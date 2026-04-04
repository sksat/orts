#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.12"
# dependencies = ["matplotlib"]
# ///
"""Plot altitude history for each scenario from orbital_lifetime CSV.

Usage:
    # Generate CSV first:
    cargo run --example orbital_lifetime -p orts --features fetch-weather
    cargo run --bin orts -- convert --format csv \
        orts/examples/orbital_lifetime/orbital_lifetime.rrd \
        --output orts/examples/orbital_lifetime/orbital_lifetime.csv

    # Plot:
    python3 orts/examples/orbital_lifetime/plot_altitude.py
"""

import math
import sys
from collections import defaultdict
from pathlib import Path

import matplotlib.pyplot as plt

R_EARTH = 6378.137  # km
OBSERVED_LIFETIME_DAYS = 78.0

# Last TLE for YODAKA (NORAD 62295) before decay.
# Source: satcat.com / Space-Track
#   Epoch: 2025-02-24T13:40:39Z  (day 77.2 after deployment 2024-12-09T08:15Z)
#   Apogee: 196 km, Perigee: 185 km
#   Mean motion: 16.30980865 rev/day
LAST_TLE_DAY = 77.2
LAST_TLE_APOGEE = 196.0
LAST_TLE_PERIGEE = 185.0
LAST_TLE_MEAN_ALT = (LAST_TLE_APOGEE + LAST_TLE_PERIGEE) / 2.0

SCENARIO_LABELS = {
    "/world/sat/scenario_a": "A: Exponential (B=0.015)",
    "/world/sat/scenario_b": "B: Harris-Priester (B=0.015)",
    "/world/sat/scenario_c": "C: NRLMSISE-00+Const (B=0.015)",
    "/world/sat/scenario_d": "D: MSISE+CSSI@launch (B=0.015)",
    "/world/sat/scenario_e": "E: MSISE+CSSI@launch (B=0.012)",
    "/world/sat/scenario_f": "F: MSISE+CSSI@launch (B=0.018)",
    "/world/sat/scenario_g": "G: MSISE+CSSI full (B=0.015)",
    "/world/sat/scenario_h": "H: MSISE+CSSI full (B=0.018)",
}

GROUP_COLORS = {
    "/world/sat/scenario_a": "#aec7e8",  # light blue
    "/world/sat/scenario_b": "#ffbb78",  # light orange
    "/world/sat/scenario_c": "#98df8a",  # light green
    "/world/sat/scenario_d": "#d62728",  # red
    "/world/sat/scenario_e": "#9467bd",  # purple
    "/world/sat/scenario_f": "#e377c2",  # pink
    "/world/sat/scenario_g": "#17becf",  # cyan
    "/world/sat/scenario_h": "#2ca02c",  # green
}

GROUP_STYLES = {
    "/world/sat/scenario_a": ":",   # predictive: dotted
    "/world/sat/scenario_b": ":",
    "/world/sat/scenario_c": "--",  # predictive + NRLMSISE: dashed
    "/world/sat/scenario_d": "-",   # launch-day: solid
    "/world/sat/scenario_e": "-",
    "/world/sat/scenario_f": "-",
    "/world/sat/scenario_g": "-.",  # retrospective: dash-dot
    "/world/sat/scenario_h": "-.",
}

GROUP_WIDTHS = {
    "/world/sat/scenario_a": 1.0,
    "/world/sat/scenario_b": 1.0,
    "/world/sat/scenario_c": 1.2,
    "/world/sat/scenario_d": 2.0,
    "/world/sat/scenario_e": 1.5,
    "/world/sat/scenario_f": 1.5,
    "/world/sat/scenario_g": 1.5,
    "/world/sat/scenario_h": 1.5,
}


def load_csv(path: str):
    """Load CSV and return {entity: [(day, altitude_km), ...]}."""
    data = defaultdict(list)

    with open(path) as f:
        for line in f:
            if line.startswith("#"):
                continue
            parts = line.strip().split(",")
            if len(parts) < 5:
                continue
            entity = parts[0]
            if not entity.startswith("/world/sat/"):
                continue
            t_s = float(parts[1])
            x, y, z = float(parts[2]), float(parts[3]), float(parts[4])
            r = math.sqrt(x * x + y * y + z * z)
            alt = r - R_EARTH
            day = t_s / 86400.0
            data[entity].append((day, alt))

    return data


def main():
    csv_path = Path(__file__).parent / "orbital_lifetime.csv"
    if not csv_path.exists():
        print(f"CSV not found: {csv_path}", file=sys.stderr)
        print("Run the example and convert first (see docstring).", file=sys.stderr)
        sys.exit(1)

    data = load_csv(str(csv_path))

    fig, ax = plt.subplots(figsize=(13, 7))

    # Plot simulation scenarios
    for entity in sorted(data.keys()):
        points = data[entity]
        days = [p[0] for p in points]
        alts = [p[1] for p in points]
        label = SCENARIO_LABELS.get(entity, entity)
        color = GROUP_COLORS.get(entity, "gray")
        style = GROUP_STYLES.get(entity, "-")
        width = GROUP_WIDTHS.get(entity, 1.0)
        ax.plot(days, alts, style, color=color, label=label, linewidth=width)

    # Last TLE observation point (apogee/perigee band + mean)
    ax.errorbar(
        LAST_TLE_DAY,
        LAST_TLE_MEAN_ALT,
        yerr=[[LAST_TLE_MEAN_ALT - LAST_TLE_PERIGEE], [LAST_TLE_APOGEE - LAST_TLE_MEAN_ALT]],
        fmt="ko",
        markersize=6,
        capsize=4,
        capthick=1.5,
        linewidth=1.5,
        label=f"Last TLE (day {LAST_TLE_DAY:.0f}, {LAST_TLE_PERIGEE:.0f}-{LAST_TLE_APOGEE:.0f} km)",
        zorder=10,
    )

    # Observed reentry line
    ax.axvline(
        x=OBSERVED_LIFETIME_DAYS,
        color="black",
        linestyle=":",
        linewidth=1.5,
        label=f"Observed reentry ({OBSERVED_LIFETIME_DAYS:.0f} d)",
    )

    ax.set_xlabel("Days since deployment", fontsize=11)
    ax.set_ylabel("Altitude [km]", fontsize=11)
    ax.set_title(
        "AE1b (YODAKA) Orbital Lifetime Analysis\n"
        "Altitude Decay: Predictive vs Launch-day vs Retrospective",
        fontsize=12,
    )
    ax.legend(loc="upper right", fontsize=8, ncol=2)
    ax.set_xlim(0, 150)
    ax.set_ylim(80, 430)
    ax.grid(True, alpha=0.3)

    out_path = csv_path.parent / "altitude_history.png"
    fig.savefig(str(out_path), dpi=150, bbox_inches="tight")
    print(f"Saved: {out_path}")
    plt.close()


if __name__ == "__main__":
    main()
