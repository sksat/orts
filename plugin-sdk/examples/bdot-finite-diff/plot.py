#!/usr/bin/env python3
"""Plot B-dot detumbling simulation results from CSV files.

Reads `sim_<model>_gain_*_omega_*.csv` files in the current directory
and produces a gain × initial-ω matrix plot (`bdot_detumbling.png`),
overlaying TiltedDipole and IGRF-14 magnetic field models.

Run from the bdot-finite-diff directory:

    uv run plot.py
"""

import csv
import math
import re
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

HERE = Path(__file__).parent
RAD2DEG = 180.0 / math.pi

MODEL_STYLES = {
    "dipole": {"color": "C0", "linestyle": "-", "label": "TiltedDipole"},
    "igrf": {"color": "C1", "linestyle": "-", "label": "IGRF-14"},
}


def load_csvs() -> dict[tuple[str, str, str], list[tuple[float, float]]]:
    """Load all sim_*.csv files, keyed by (model, gain_label, omega_label)."""
    data: dict[tuple[str, str, str], list[tuple[float, float]]] = {}
    pattern = re.compile(r"sim_(dipole|igrf)_gain_(.+)_omega_(.+)\.csv")
    for csv_path in sorted(HERE.glob("sim_*_gain_*_omega_*.csv")):
        m = pattern.match(csv_path.name)
        if not m:
            continue
        model, gain_label, omega_label = m.group(1), m.group(2), m.group(3)
        rows: list[tuple[float, float]] = []
        with open(csv_path) as f:
            reader = csv.DictReader(f)
            for row in reader:
                rows.append((float(row["t"]), float(row["omega_mag"])))
        data[(model, gain_label, omega_label)] = rows
    return data


def plot_matrix(data: dict[tuple[str, str, str], list[tuple[float, float]]]) -> None:
    """3×3 matrix: rows = gain, columns = initial ω, overlaid models."""
    gains = sorted({g for _, g, _ in data}, reverse=True)
    omegas = sorted({o for _, _, o in data}, reverse=True)
    models = sorted({m for m, _, _ in data})
    nrows, ncols = len(gains), len(omegas)

    fig, axes = plt.subplots(
        nrows, ncols, figsize=(4.5 * ncols, 3.5 * nrows),
        sharex=True, sharey=True, squeeze=False,
    )

    # Global Y limits from all data.
    all_ws = [w for rows in data.values() for _, w in rows]
    y_lo = min(all_ws) * 0.9
    y_hi = max(all_ws) * 1.05

    for r, gain in enumerate(gains):
        for c, omega in enumerate(omegas):
            ax = axes[r][c]

            for model in models:
                key = (model, gain, omega)
                if key not in data:
                    continue
                rows = data[key]
                ts = np.array([t for t, _ in rows])
                ws = np.array([w for _, w in rows])
                style = MODEL_STYLES.get(model, {"color": "gray", "linestyle": "--", "label": model})
                ax.plot(ts, ws, color=style["color"], linestyle=style["linestyle"],
                        linewidth=1.5, label=style["label"])

                # Annotate reduction percentage (top-right, stacked).
                w0, wf = ws[0], ws[-1]
                pct = (1.0 - wf / w0) * 100.0
                y_pos = 0.82 if model == models[0] else 0.74
                ax.annotate(
                    f"{style['label']}: {pct:.0f}%",
                    xy=(0.95, y_pos), xycoords="axes fraction",
                    ha="right", va="top", fontsize=8,
                    color=style["color"],
                )

            ax.set_ylim(y_lo, y_hi)
            ax.grid(True, alpha=0.3)

            # Row label (left edge).
            if c == 0:
                ax.set_ylabel(f"k = {gain}\n|ω| [rad/s]")

            # Column label (top edge).
            if r == 0:
                ax.set_title(f"|ω₀| = {omega} rad/s ({float(omega) * RAD2DEG:.1f} deg/s)")

            # X label (bottom edge).
            if r == nrows - 1:
                ax.set_xlabel("Time [s]")

            # Legend (top-left cell only).
            if r == 0 and c == 0:
                ax.legend(fontsize=7, loc="upper right")

            # Secondary Y axis (right edge only).
            if c == ncols - 1:
                ax2 = ax.secondary_yaxis(
                    "right",
                    functions=(lambda x: x * RAD2DEG, lambda x: x / RAD2DEG),
                )
                ax2.set_ylabel("[deg/s]")

    fig.suptitle("B-dot Detumbling — WASM Guest Plugin", fontsize=14, y=1.01)
    fig.tight_layout()
    out = HERE / "bdot_detumbling.png"
    fig.savefig(out, dpi=150, bbox_inches="tight")
    print(f"Saved {out}")
    plt.close(fig)


def main() -> None:
    data = load_csvs()
    if not data:
        print("No CSV files found. Run the simulation first:")
        print("  cargo run --example wasm-bdot --features plugin-wasm --release")
        return
    plot_matrix(data)


if __name__ == "__main__":
    main()
