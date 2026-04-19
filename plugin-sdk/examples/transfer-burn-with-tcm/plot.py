#!/usr/bin/env python3
"""Plot transfer-burn-with-tcm plugin simulation results.

Reads `sim.csv` (output of `orts run --config orts.toml --output stdout
--format csv > sim.csv`) and writes `transfer_burn_with_tcm.png` with:

1. XY orbital trajectory (left) — Earth, initial/target circles, and the
   transfer arc overlaid. Segments where the thruster is firing
   (throttle > 0) are highlighted in red.
2. Altitude vs time (right top) with burn windows shaded.
3. Orbital speed |v| vs time (right bottom) with burn windows shaded — shows
   Δv added per burn and KE↔PE conversion during coast.

CSV columns used (0-indexed):
    0: t
    1-3: x, y, z [km]
    4-6: vx, vy, vz [km/s]
    26-28: throttle_0, throttle_1, throttle_2

    uv run plot.py
"""

from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

HERE = Path(__file__).parent
EARTH_RADIUS_KM = 6378.137


def main() -> None:
    csv = HERE / "sim.csv"
    if not csv.exists():
        print(f"No CSV at {csv}. Run:")
        print("  orts run --config orts.toml --output stdout --format csv > sim.csv")
        return

    data = np.loadtxt(csv, delimiter=",", comments="#")
    t = data[:, 0]
    pos = data[:, 1:4]
    vel = data[:, 4:7]
    throttle0 = data[:, 26]

    altitude = np.linalg.norm(pos, axis=1) - EARTH_RADIUS_KM
    speed = np.linalg.norm(vel, axis=1)
    burning = throttle0 > 0.0
    burn_intervals = _find_burn_intervals(t, burning)

    fig = plt.figure(figsize=(13, 6.5))
    gs = fig.add_gridspec(2, 2, width_ratios=[1.1, 1.0], hspace=0.3, wspace=0.25)
    ax_traj = fig.add_subplot(gs[:, 0])
    ax_alt = fig.add_subplot(gs[0, 1])
    ax_v = fig.add_subplot(gs[1, 1], sharex=ax_alt)

    # --- trajectory ---
    earth = plt.Circle((0, 0), EARTH_RADIUS_KM, color="#4477AA", alpha=0.6, zorder=0)
    ax_traj.add_patch(earth)
    theta = np.linspace(0, 2 * np.pi, 200)
    r_init = EARTH_RADIUS_KM + altitude[0]
    r_final = EARTH_RADIUS_KM + altitude[-1]
    ax_traj.plot(r_init * np.cos(theta), r_init * np.sin(theta),
                 "--", color="gray", linewidth=0.6, alpha=0.6, zorder=1,
                 label="initial circle")
    ax_traj.plot(r_final * np.cos(theta), r_final * np.sin(theta),
                 ":", color="gray", linewidth=0.8, alpha=0.6, zorder=1,
                 label="target circle")
    # Coast (throttle = 0)
    coast_mask = ~burning
    ax_traj.plot(
        np.where(coast_mask, pos[:, 0], np.nan),
        np.where(coast_mask, pos[:, 1], np.nan),
        color="C1", linewidth=1.0, zorder=2, label="coast",
    )
    # Burn (throttle > 0) — highlight in red (thicker line)
    burn_mask = burning
    ax_traj.plot(
        np.where(burn_mask, pos[:, 0], np.nan),
        np.where(burn_mask, pos[:, 1], np.nan),
        color="C3", linewidth=3.5, zorder=3, label="burn",
    )
    ax_traj.plot(pos[0, 0], pos[0, 1], "o", color="C2", markersize=6, zorder=4,
                 label=f"start ({altitude[0]:.0f} km)")
    ax_traj.plot(pos[-1, 0], pos[-1, 1], "s", color="C0", markersize=6, zorder=4,
                 label=f"end ({altitude[-1]:.0f} km)")
    ax_traj.set_aspect("equal")
    ax_traj.set_xlabel("X [km]")
    ax_traj.set_ylabel("Y [km]")
    ax_traj.set_title("ECI XY trajectory — burns highlighted in red")
    ax_traj.legend(loc="lower right", fontsize=8)
    ax_traj.grid(True, alpha=0.3)

    # --- altitude ---
    for (t0, t1) in burn_intervals:
        ax_alt.axvspan(t0, t1, color="C3", alpha=0.22, zorder=0)
    ax_alt.plot(t, altitude, color="C0", linewidth=1.5, zorder=2)
    ax_alt.axhline(altitude[0], color="gray", linestyle=":", linewidth=0.8,
                   label=f"initial = {altitude[0]:.0f} km", zorder=1)
    _annotate_burns(ax_alt, altitude, t, burn_intervals, "Δalt")
    ax_alt.set_ylabel("Altitude [km]")
    ax_alt.set_title("Altitude and orbital speed vs time (red = burn)")
    ax_alt.legend(loc="lower right", fontsize=8)
    ax_alt.grid(True, alpha=0.3)

    # --- speed ---
    for (t0, t1) in burn_intervals:
        ax_v.axvspan(t0, t1, color="C3", alpha=0.22, zorder=0)
    ax_v.plot(t, speed, color="C3", linewidth=1.5, zorder=2)
    ax_v.axhline(speed[0], color="gray", linestyle=":", linewidth=0.8,
                 label=f"initial = {speed[0]:.3f} km/s", zorder=1)
    _annotate_burns(ax_v, speed, t, burn_intervals, "Δv",
                    fmt=lambda d: f"{d * 1000:+.0f} m/s")
    ax_v.set_ylabel("Orbital speed |v| [km/s]")
    ax_v.set_xlabel("Time [s]")
    ax_v.legend(loc="lower left", fontsize=8)
    ax_v.grid(True, alpha=0.3)

    fig.suptitle(
        "Transfer Burn with TCM — WASM Plugin (RW attitude tracking + thruster)",
        y=0.98,
    )
    fig.tight_layout(rect=(0, 0, 1, 0.96))
    out = HERE / "transfer_burn_with_tcm.png"
    fig.savefig(out, dpi=150)
    print(f"Saved {out}")
    plt.close(fig)


def _find_burn_intervals(t, burning):
    """Return list of (start_time, end_time) for each contiguous burn window."""
    intervals = []
    in_burn = False
    start = None
    for i, b in enumerate(burning):
        if b and not in_burn:
            start = t[i]
            in_burn = True
        elif not b and in_burn:
            intervals.append((start, t[i - 1]))
            in_burn = False
    if in_burn:
        intervals.append((start, t[-1]))
    return intervals


def _annotate_burns(ax, y, t, intervals, label, fmt=None):
    """Label each burn window with the change in `y` across it."""
    if fmt is None:
        fmt = lambda d: f"{d:+.1f}"
    for (t0, t1) in intervals:
        i0 = int(np.searchsorted(t, t0))
        i1 = int(np.searchsorted(t, t1))
        i1 = min(i1, len(y) - 1)
        dy = y[i1] - y[i0]
        # position label near the center-top of the burn window
        x_mid = 0.5 * (t0 + t1)
        y_top = ax.get_ylim()[1]
        ax.annotate(
            f"{label}={fmt(dy)}",
            xy=(x_mid, y_top),
            xytext=(0, -12),
            textcoords="offset points",
            ha="center",
            fontsize=8,
            color="C3",
        )


if __name__ == "__main__":
    main()
