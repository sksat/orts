#!/usr/bin/env python3
"""Plot PD+RW attitude control simulation results.

Reads `sim_pd_rw.csv` and produces `pd_rw_control.png` with three subplots:
1. Attitude error [deg] vs time
2. Angular velocity components [rad/s] vs time
3. RW wheel momentum [N·m·s] vs time

    uv run plot.py
"""

import csv
from pathlib import Path

import matplotlib.pyplot as plt

HERE = Path(__file__).parent


def load_csv() -> dict[str, list[float]]:
    path = HERE / "sim_pd_rw.csv"
    if not path.exists():
        return {}
    data: dict[str, list[float]] = {}
    with open(path) as f:
        reader = csv.DictReader(f)
        for row in reader:
            for key, val in row.items():
                data.setdefault(key, []).append(float(val))
    return data


def main() -> None:
    data = load_csv()
    if not data:
        print("No CSV found. Run the simulation first:")
        print("  cargo run --example wasm_pd_rw_simulate --features plugin-wasm --release")
        return

    t = data["t"]

    fig, axes = plt.subplots(3, 1, figsize=(10, 9), sharex=True)

    # 1. Attitude error
    ax = axes[0]
    ax.plot(t, data["angle_error_deg"], color="C0", linewidth=1.5)
    ax.set_ylabel("Attitude error [deg]")
    ax.set_title("PD + Reaction Wheel Attitude Control (WASM Guest)")
    ax.grid(True, alpha=0.3)

    # 2. Angular velocity
    ax = axes[1]
    ax.plot(t, data["omega_x"], label="ωx", linewidth=1)
    ax.plot(t, data["omega_y"], label="ωy", linewidth=1)
    ax.plot(t, data["omega_z"], label="ωz", linewidth=1)
    ax.plot(t, data["omega_mag"], label="|ω|", linewidth=1.5, color="k", linestyle="--")
    ax.set_ylabel("Angular velocity [rad/s]")
    ax.legend(fontsize=8, ncol=4)
    ax.grid(True, alpha=0.3)

    # 3. RW momentum
    ax = axes[2]
    ax.plot(t, data["h_x"], label="hx", linewidth=1)
    ax.plot(t, data["h_y"], label="hy", linewidth=1)
    ax.plot(t, data["h_z"], label="hz", linewidth=1)
    ax.plot(t, data["h_mag"], label="|h|", linewidth=1.5, color="k", linestyle="--")
    ax.set_ylabel("RW momentum [N·m·s]")
    ax.set_xlabel("Time [s]")
    ax.legend(fontsize=8, ncol=4)
    ax.grid(True, alpha=0.3)

    fig.tight_layout()
    out = HERE / "pd_rw_control.png"
    fig.savefig(out, dpi=150)
    print(f"Saved {out}")
    plt.close(fig)


if __name__ == "__main__":
    main()
