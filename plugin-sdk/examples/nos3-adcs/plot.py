#!/usr/bin/env python3
"""Plot NOS3 ADCS simulation results from CSV output.

Usage:
    # Generate CSV first:
    orts run --config orts.toml --format csv --output stdout > result.csv
    # Then plot:
    python3 plot.py result.csv
    python3 plot.py result.csv --save output.png
"""

import argparse
import csv
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np


def parse_csv(path: str) -> dict:
    """Parse orts CSV output into a dict of numpy arrays."""
    metadata = {}
    header = []
    rows = []

    with open(path) as f:
        for line in f:
            line = line.strip()
            if line.startswith("# ") and "," in line and line.startswith("# t[s]"):
                # Header line
                header = [h.strip() for h in line[2:].split(",")]
            elif line.startswith("#"):
                # Metadata comment
                if "=" in line:
                    k, v = line[2:].split("=", 1)
                    metadata[k.strip()] = v.strip()
            else:
                rows.append([float(x) for x in line.split(",")])

    if not header or not rows:
        print("Error: no data found in CSV", file=sys.stderr)
        sys.exit(1)

    data = np.array(rows)
    return {name: data[:, i] for i, name in enumerate(header)}


def plot_results(data: dict, title: str = "NOS3 ADCS Simulation"):
    """Create a multi-panel plot of attitude control results."""
    t = data["t[s]"]

    has_omega = "wx" in data
    has_quat = "qw" in data
    has_mtq = "mtq_mx" in data
    has_rw_cmd = "rw_tx" in data and np.any(np.abs(data["rw_tx"]) > 1e-15)
    has_rw_mom = "rw_hx" in data and np.any(np.abs(data["rw_hx"]) > 1e-15)

    # Count panels (skip empty actuator channels)
    panels = []
    if has_omega:
        panels.append("omega")
    if has_quat:
        panels.append("quat")
    if has_mtq:
        panels.append("mtq")
    if has_rw_cmd:
        panels.append("rw_cmd")
    if has_rw_mom:
        panels.append("rw_mom")

    n = len(panels)
    if n == 0:
        print("No attitude/command data found", file=sys.stderr)
        sys.exit(1)

    fig, axes = plt.subplots(n, 1, figsize=(12, 3 * n), sharex=True)
    if n == 1:
        axes = [axes]

    for ax, panel in zip(axes, panels):
        if panel == "omega":
            wx, wy, wz = data["wx"], data["wy"], data["wz"]
            norm = np.sqrt(wx**2 + wy**2 + wz**2)
            ax.plot(t, wx, label=r"$\omega_x$", alpha=0.7)
            ax.plot(t, wy, label=r"$\omega_y$", alpha=0.7)
            ax.plot(t, wz, label=r"$\omega_z$", alpha=0.7)
            ax.plot(t, norm, "k--", label=r"$|\omega|$", linewidth=1.5)
            ax.set_ylabel("Angular velocity [rad/s]")
            ax.legend(loc="upper right", ncol=4, fontsize=8)
            ax.grid(True, alpha=0.3)
            # Secondary y-axis in deg/s
            ax2 = ax.twinx()
            ax2.set_ylabel("[deg/s]")
            rad2deg = 180.0 / np.pi
            ax2.set_ylim(ax.get_ylim()[0] * rad2deg, ax.get_ylim()[1] * rad2deg)

        elif panel == "quat":
            ax.plot(t, data["qw"], label=r"$q_w$")
            ax.plot(t, data["qx"], label=r"$q_x$", alpha=0.7)
            ax.plot(t, data["qy"], label=r"$q_y$", alpha=0.7)
            ax.plot(t, data["qz"], label=r"$q_z$", alpha=0.7)
            ax.set_ylabel("Quaternion")
            ax.legend(loc="upper right", ncol=4, fontsize=8)
            ax.grid(True, alpha=0.3)

        elif panel == "mtq":
            ax.plot(t, data["mtq_mx"], label=r"$m_x$", alpha=0.7)
            ax.plot(t, data["mtq_my"], label=r"$m_y$", alpha=0.7)
            ax.plot(t, data["mtq_mz"], label=r"$m_z$", alpha=0.7)
            ax.set_ylabel(r"MTQ command [A$\cdot$m$^2$]")
            ax.legend(loc="upper right", ncol=3, fontsize=8)
            ax.grid(True, alpha=0.3)

        elif panel == "rw_cmd":
            ax.plot(t, data["rw_tx"], label=r"$\tau_x$", alpha=0.7)
            ax.plot(t, data["rw_ty"], label=r"$\tau_y$", alpha=0.7)
            ax.plot(t, data["rw_tz"], label=r"$\tau_z$", alpha=0.7)
            ax.set_ylabel(r"RW torque cmd [N$\cdot$m]")
            ax.legend(loc="upper right", ncol=3, fontsize=8)
            ax.grid(True, alpha=0.3)

        elif panel == "rw_mom":
            ax.plot(t, data["rw_hx"], label=r"$h_x$", alpha=0.7)
            ax.plot(t, data["rw_hy"], label=r"$h_y$", alpha=0.7)
            ax.plot(t, data["rw_hz"], label=r"$h_z$", alpha=0.7)
            ax.set_ylabel(r"RW momentum [N$\cdot$m$\cdot$s]")
            ax.legend(loc="upper right", ncol=3, fontsize=8)
            ax.grid(True, alpha=0.3)

    axes[-1].set_xlabel("Time [s]")
    fig.suptitle(title, fontsize=14)
    fig.tight_layout()
    return fig


def main():
    parser = argparse.ArgumentParser(description="Plot NOS3 ADCS simulation results")
    parser.add_argument("csv", help="CSV file from orts run --format csv")
    parser.add_argument("--save", help="Save plot to file (e.g. output.png)")
    parser.add_argument("--title", default="NOS3 ADCS Simulation", help="Plot title")
    args = parser.parse_args()

    data = parse_csv(args.csv)
    fig = plot_results(data, title=args.title)

    if args.save:
        fig.savefig(args.save, dpi=150, bbox_inches="tight")
        print(f"Saved to {args.save}")
    else:
        plt.show()


if __name__ == "__main__":
    main()
