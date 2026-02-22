# /// script
# requires-python = ">=3.10"
# dependencies = ["pymsis", "numpy"]
# ///
"""Generate NRLMSISE-00 density reference fixtures using pymsis.

pymsis wraps the official NRL Fortran code for NRLMSISE-00 (version=0),
providing a direct oracle for validating our clean-room Rust implementation.

Output indices (pymsis.Variable):
  0: MASS_DENSITY   [kg/m³]
  1: N2             [m⁻³]
  2: O2             [m⁻³]
  3: O              [m⁻³]
  4: HE             [m⁻³]
  5: H              [m⁻³]
  6: AR             [m⁻³]
  7: N              [m⁻³]
  8: ANOMALOUS_O    [m⁻³]
  9: NO             [m⁻³]  (only in MSIS 2.x; 0.0 for NRLMSISE-00)
 10: TEMPERATURE    [K]

Run:  uv run tools/generate_nrlmsise00_fixtures.py
"""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import numpy as np


def generate_fixtures():
    """Generate NRLMSISE-00 reference fixtures."""
    import pymsis

    # Verify we can call version=0 (NRLMSISE-00)
    test = pymsis.calculate(
        np.datetime64("2024-03-20T12:00"),
        0.0,
        0.0,
        400.0,
        f107s=150.0,
        f107as=150.0,
        aps=[[15.0] * 7],
        version=0,
    )
    assert test.shape[-1] == 11, f"Expected 11 output variables, got {test.shape[-1]}"
    print(f"pymsis version check OK: shape={test.shape}, rho={test.flat[0]:.6e} kg/m³")

    # ── Condition axes ──

    # Altitudes [km]: span full MSIS range
    altitudes = [100.0, 150.0, 200.0, 300.0, 400.0, 500.0, 700.0, 1000.0]

    # Geographic coordinates
    latitudes = [0.0, 45.0, 75.0, -45.0]  # equator, mid, high, southern
    longitudes = [0.0, 90.0, 180.0, 270.0]  # midnight, dawn, noon, dusk (approx LST)

    # Epochs (fixed dates for reproducibility)
    epochs = [
        ("2024-03-20T12:00:00Z", "vernal_equinox"),
        ("2024-06-21T12:00:00Z", "summer_solstice"),
        ("2024-12-21T12:00:00Z", "winter_solstice"),
    ]

    # Solar/geomagnetic activity levels
    activity_levels = [
        {"name": "solar_min", "f107": 70.0, "f107a": 70.0, "ap": 4.0},
        {"name": "solar_moderate", "f107": 150.0, "f107a": 150.0, "ap": 15.0},
        {"name": "solar_max", "f107": 250.0, "f107a": 250.0, "ap": 50.0},
    ]

    # Variable names matching pymsis.Variable order (indices 0-10)
    # Index 9 (NO) is NaN for NRLMSISE-00 (v0), only available in MSIS 2.x
    all_var_names = [
        "mass_density_kg_m3",  # 0
        "n2_m3",               # 1
        "o2_m3",               # 2
        "o_m3",                # 3
        "he_m3",               # 4
        "h_m3",                # 5
        "ar_m3",               # 6
        "n_m3",                # 7
        "anomalous_o_m3",      # 8
        # 9: NO (skip — NaN for v0)
        "temperature_k",       # 10
    ]
    # Indices to extract (skip index 9 = NO)
    var_indices = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10]

    fixture = {
        "generator": "tools/generate_nrlmsise00_fixtures.py",
        "oracle": "pymsis (NRL official Fortran, version=0 = NRLMSISE-00)",
        "pymsis_version": pymsis.__version__,
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "variable_names": all_var_names,
        "points": [],
    }

    n_total = (
        len(altitudes) * len(latitudes) * len(longitudes)
        * len(epochs) * len(activity_levels)
    )
    print(f"Generating {n_total} data points...")

    count = 0
    for epoch_str, epoch_name in epochs:
        date = np.datetime64(epoch_str[:-1])  # strip Z for numpy

        for activity in activity_levels:
            f107 = activity["f107"]
            f107a = activity["f107a"]
            ap = activity["ap"]
            ap_array = [[ap] * 7]  # daily Ap for all 7 slots

            for lat in latitudes:
                for lon in longitudes:
                    for alt in altitudes:
                        result = pymsis.calculate(
                            date,
                            lon,
                            lat,
                            alt,
                            f107s=f107,
                            f107as=f107a,
                            aps=ap_array,
                            version=0,
                        )
                        # result shape: (1, 11) in satellite mode
                        raw = result.flatten()

                        point = {
                            "epoch_utc": epoch_str,
                            "epoch_name": epoch_name,
                            "activity": activity["name"],
                            "f107": f107,
                            "f107a": f107a,
                            "ap": ap,
                            "latitude_deg": lat,
                            "longitude_deg": lon,
                            "altitude_km": alt,
                        }
                        # Add output variables (skip NO which is NaN for v0)
                        for idx, name in zip(var_indices, all_var_names):
                            v = float(raw[idx])
                            point[name] = None if np.isnan(v) else v

                        fixture["points"].append(point)
                        count += 1

            print(
                f"  {epoch_name} / {activity['name']}: "
                f"{count}/{n_total} points"
            )

    # ── Exospheric temperature estimation ──
    # Evaluate at 2000 km where T ≈ T_exo for a subset of conditions
    print("Generating exospheric temperature reference points...")
    exo_points = []
    for epoch_str, epoch_name in epochs:
        date = np.datetime64(epoch_str[:-1])
        for activity in activity_levels:
            ap_array = [[activity["ap"]] * 7]
            # Equatorial, noon
            result = pymsis.calculate(
                date,
                180.0,
                0.0,
                2000.0,
                f107s=activity["f107"],
                f107as=activity["f107a"],
                aps=ap_array,
                version=0,
            )
            values = result.flatten().tolist()
            exo_points.append({
                "epoch_utc": epoch_str,
                "epoch_name": epoch_name,
                "activity": activity["name"],
                "f107": activity["f107"],
                "f107a": activity["f107a"],
                "ap": activity["ap"],
                "latitude_deg": 0.0,
                "longitude_deg": 180.0,
                "altitude_km": 2000.0,
                "temperature_k": values[10],
                "mass_density_kg_m3": values[0],
            })

    fixture["exospheric_temperature_points"] = exo_points

    # ── Summary statistics ──
    densities = [p["mass_density_kg_m3"] for p in fixture["points"]]
    temps = [p["temperature_k"] for p in fixture["points"]]
    fixture["summary"] = {
        "total_points": len(fixture["points"]),
        "exo_temp_points": len(exo_points),
        "density_range_kg_m3": [min(densities), max(densities)],
        "temperature_range_k": [min(temps), max(temps)],
        "altitudes_km": altitudes,
        "latitudes_deg": latitudes,
        "longitudes_deg": longitudes,
        "epochs": [e[0] for e in epochs],
        "activity_levels": [a["name"] for a in activity_levels],
    }

    return fixture


def main():
    fixture = generate_fixtures()

    out_path = Path("tobari/tests/fixtures/nrlmsise00_reference.json")
    out_path.parent.mkdir(parents=True, exist_ok=True)

    with open(out_path, "w") as f:
        json.dump(fixture, f, indent=2)

    size_kb = out_path.stat().st_size / 1024
    print(f"\nWrote {out_path} ({size_kb:.1f} KB)")
    print(f"  {fixture['summary']['total_points']} density points")
    print(f"  {fixture['summary']['exo_temp_points']} exospheric temperature points")
    print(
        f"  Density range: {fixture['summary']['density_range_kg_m3'][0]:.2e} "
        f"to {fixture['summary']['density_range_kg_m3'][1]:.2e} kg/m³"
    )
    print(
        f"  Temperature range: {fixture['summary']['temperature_range_k'][0]:.1f} "
        f"to {fixture['summary']['temperature_range_k'][1]:.1f} K"
    )


if __name__ == "__main__":
    main()
