#!/usr/bin/env python3
"""Plot B-dot detumbling simulation results from CSV files.

Reads `sim_gain_*_omega_*.csv` files in the current directory and
produces:

- `bdot_gain_sweep.png`: |ω| vs time for all gain values (one subplot
  per initial angular velocity).
- `bdot_omega_sweep.png`: |ω| vs time for all initial angular velocities
  (one subplot per gain).

Run from the bdot-finite-diff directory:

    uv run plot.py
"""

import csv
import re
from collections import defaultdict
from pathlib import Path

import matplotlib.pyplot as plt

HERE = Path(__file__).parent


def load_csvs() -> dict[tuple[str, str], list[tuple[float, float]]]:
    """Load all sim_*.csv files, keyed by (gain_label, omega_label)."""
    data: dict[tuple[str, str], list[tuple[float, float]]] = {}
    pattern = re.compile(r"sim_gain_(.+)_omega_(.+)\.csv")
    for csv_path in sorted(HERE.glob("sim_gain_*_omega_*.csv")):
        m = pattern.match(csv_path.name)
        if not m:
            continue
        gain_label, omega_label = m.group(1), m.group(2)
        rows: list[tuple[float, float]] = []
        with open(csv_path) as f:
            reader = csv.DictReader(f)
            for row in reader:
                rows.append((float(row["t"]), float(row["omega_mag"])))
        data[(gain_label, omega_label)] = rows
    return data


def plot_gain_sweep(data: dict[tuple[str, str], list[tuple[float, float]]]) -> None:
    """One subplot per initial ω, lines for each gain."""
    # Group by omega.
    omegas: dict[str, list[tuple[str, list[tuple[float, float]]]]] = defaultdict(list)
    for (gain, omega), rows in sorted(data.items()):
        omegas[omega].append((gain, rows))

    n = len(omegas)
    fig, axes = plt.subplots(1, n, figsize=(5 * n, 4), sharey=False, squeeze=False)
    for ax, (omega, series_list) in zip(axes[0], sorted(omegas.items())):
        for gain, rows in series_list:
            ts = [r[0] for r in rows]
            ws = [r[1] for r in rows]
            ax.plot(ts, ws, label=f"k = {gain}")
        ax.set_xlabel("Time [s]")
        ax.set_ylabel("|ω| [rad/s]")
        ax.set_title(f"Initial |ω| = {omega} rad/s")
        ax.legend(fontsize=8)
        ax.grid(True, alpha=0.3)

    fig.suptitle("B-dot Finite-Diff Detumbling — Gain Sweep (WASM Guest)", fontsize=12)
    fig.tight_layout()
    out = HERE / "bdot_gain_sweep.png"
    fig.savefig(out, dpi=150)
    print(f"Saved {out}")
    plt.close(fig)


def plot_omega_sweep(data: dict[tuple[str, str], list[tuple[float, float]]]) -> None:
    """One subplot per gain, lines for each initial ω."""
    gains: dict[str, list[tuple[str, list[tuple[float, float]]]]] = defaultdict(list)
    for (gain, omega), rows in sorted(data.items()):
        gains[gain].append((omega, rows))

    n = len(gains)
    fig, axes = plt.subplots(1, n, figsize=(5 * n, 4), sharey=False, squeeze=False)
    for ax, (gain, series_list) in zip(axes[0], sorted(gains.items())):
        for omega, rows in series_list:
            ts = [r[0] for r in rows]
            ws = [r[1] for r in rows]
            ax.plot(ts, ws, label=f"|ω₀| = {omega}")
        ax.set_xlabel("Time [s]")
        ax.set_ylabel("|ω| [rad/s]")
        ax.set_title(f"Gain k = {gain}")
        ax.legend(fontsize=8)
        ax.grid(True, alpha=0.3)

    fig.suptitle("B-dot Finite-Diff Detumbling — Initial ω Sweep (WASM Guest)", fontsize=12)
    fig.tight_layout()
    out = HERE / "bdot_omega_sweep.png"
    fig.savefig(out, dpi=150)
    print(f"Saved {out}")
    plt.close(fig)


def main() -> None:
    data = load_csvs()
    if not data:
        print("No CSV files found. Run the simulation first:")
        print("  cargo run --example wasm-bdot --features plugin-wasm --release")
        return
    plot_gain_sweep(data)
    plot_omega_sweep(data)


if __name__ == "__main__":
    main()
