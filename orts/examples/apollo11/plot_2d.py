# /// script
# requires-python = ">=3.10"
# dependencies = ["matplotlib>=3.8", "numpy>=1.24", "pillow>=10.0"]
# ///
"""Apollo 11 — 2D visualization (matplotlib).

Usage: uv run orts/examples/apollo11/plot_2d.py
"""
import sys
from datetime import timedelta
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.animation import FuncAnimation, PillowWriter

sys.path.insert(0, str(Path(__file__).parent))
from plot_common import (EPOCH_UTC, OUTPUT_DIR, R_EARTH, R_MOON,
                         compute_derived, load_data)


def draw_panels(axes, d, idx):
    sl = slice(0, idx + 1)

    ax = axes[0]
    ax.clear()
    ax.plot(d["sx"][sl] / 1000, d["sy"][sl] / 1000, "b-", lw=0.4, alpha=0.7)
    ax.plot(d["mx"][sl] / 1000, d["my"][sl] / 1000, "gray", lw=0.3, alpha=0.3)
    ax.plot(0, 0, "o", color="#4488ff", ms=7)
    ax.plot(d["sx"][idx] / 1000, d["sy"][idx] / 1000, "bo", ms=3)
    ax.plot(d["mx"][idx] / 1000, d["my"][idx] / 1000, "o", color="gray", ms=4)
    lim = max(abs(d["sx"]).max(), abs(d["sy"]).max()) / 1000 * 1.1
    ax.set_xlim(-lim, lim); ax.set_ylim(-lim, lim); ax.set_aspect("equal")
    ax.set_xlabel("X (×1000 km)"); ax.set_ylabel("Y (×1000 km)")
    ax.set_title("ECI XY"); ax.grid(True, alpha=0.2)

    ax = axes[1]
    ax.clear()
    ax.plot(d["rot_x"][sl], d["rot_y"][sl], "b-", lw=0.4, alpha=0.7)
    ax.plot(0, 0, "o", color="#4488ff", ms=7, label="Earth")
    ax.plot(1, 0, "o", color="#aaa", ms=5, label="Moon")
    ax.plot(d["rot_x"][idx], d["rot_y"][idx], "bo", ms=3)
    ax.set_xlim(-0.3, 1.5); ax.set_ylim(-0.6, 0.6); ax.set_aspect("equal")
    ax.set_xlabel("Earth–Moon axis"); ax.set_ylabel("Perpendicular")
    ax.set_title("Rotating Frame"); ax.legend(fontsize=7, loc="upper left"); ax.grid(True, alpha=0.2)

    ax = axes[2]
    ax.clear()
    mcx, mcy = d["mc_x"][sl] / 1000, d["mc_y"][sl] / 1000
    th = np.linspace(0, 2 * np.pi, 100)
    ax.fill(R_MOON * np.cos(th) / 1000, R_MOON * np.sin(th) / 1000, color="#ccc", alpha=0.5)
    ax.plot(R_MOON * np.cos(th) / 1000, R_MOON * np.sin(th) / 1000, color="#888", lw=1)
    ax.plot(mcx, mcy, "b-", lw=0.4, alpha=0.7)
    ax.plot(mcx[-1], mcy[-1], "bo", ms=3)
    ax.set_xlim(-15, 15); ax.set_ylim(-15, 15); ax.set_aspect("equal")
    ax.set_xlabel("X (×1000 km)"); ax.set_ylabel("Y (×1000 km)")
    ax.set_title("Moon-centered"); ax.grid(True, alpha=0.2)

    ax = axes[3]
    ax.clear()
    ax.semilogy(d["t_h"][sl], d["r_earth"][sl], "b-", lw=0.8, label="Earth")
    ax.semilogy(d["t_h"][sl], d["r_moon"][sl], "r-", lw=0.8, label="Moon")
    ax.axhline(y=R_EARTH, color="cyan", ls="--", alpha=0.3, lw=0.5)
    ax.axhline(y=R_MOON, color="gray", ls="--", alpha=0.3, lw=0.5)
    ax.set_xlim(0, d["t_h"][-1]); ax.set_ylim(1e2, 1e6)
    ax.set_xlabel("Time (hours)"); ax.set_ylabel("Distance (km)")
    ax.set_title("Distances"); ax.legend(fontsize=7); ax.grid(True, alpha=0.2)


def plot_static(d):
    fig, axes = plt.subplots(2, 2, figsize=(14, 11))
    draw_panels(list(axes.flat), d, len(d["sx"]) - 1)
    end_utc = EPOCH_UTC + timedelta(seconds=float(d["t_h"][-1] * 3600))
    fig.suptitle(
        f"Apollo 11 Trajectory — orts simulation\n"
        f"{EPOCH_UTC.strftime('%Y-%m-%d %H:%M')} – {end_utc.strftime('%Y-%m-%d %H:%M UTC')}",
        fontsize=13,
    )
    plt.tight_layout()
    out = OUTPUT_DIR / "apollo11_trajectory.png"
    plt.savefig(out, dpi=150)
    print(f"Saved {out}")
    plt.close(fig)


def plot_animation(d):
    fig, axes_a = plt.subplots(2, 2, figsize=(14, 11))
    fig.suptitle("Apollo 11 — orts simulation", fontsize=13)
    N = len(d["sx"])
    frames = 400
    idx_map = np.linspace(0, N - 1, frames).astype(int)
    time_text = fig.text(0.5, 0.02, "", ha="center", fontsize=11)

    def update(frame):
        i = idx_map[frame]
        draw_panels(list(axes_a.flat), d, i)
        utc = EPOCH_UTC + timedelta(seconds=float(d["t_h"][i] * 3600))
        time_text.set_text(
            f"GET {d['t_h'][i]:.1f}h  ({d['t_h'][i]/24:.1f} days)"
            f"    {utc.strftime('%Y-%m-%d %H:%M UTC')}"
        )

    update(0)
    plt.tight_layout(rect=[0, 0.05, 1, 0.95])
    anim = FuncAnimation(fig, update, frames=frames, interval=50)
    out = OUTPUT_DIR / "apollo11_animation.gif"
    anim.save(out, writer=PillowWriter(fps=20))
    print(f"Saved {out}")
    plt.close("all")


if __name__ == "__main__":
    sat, moon = load_data()
    d = compute_derived(sat, moon)
    plot_static(d)
    plot_animation(d)
