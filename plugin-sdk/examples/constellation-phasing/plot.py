#!/usr/bin/env python3
"""Plot constellation-phasing plugin simulation results.

Reads a multi-sat CSV produced by `orts convert --format csv ... --output FILE`
and writes `<stem>.png` next to the CSV, showing the usual per-sat trajectory
grid + SMA / altitude / phase timelines. Generalizes to any number of
satellites: the per-sat grid is only drawn for ≤ 8 sats; beyond that, only
the overlay timelines are produced.

CSV columns (0-indexed):
    0: sat_id (string like "sat-0")
    1: t [s]
    2-4: x, y, z [km] (ECI)
    5-7: vx, vy, vz [km/s]
    8: SMA [km]

    python3 plot.py                  # default: phasing.csv → constellation_phasing.png
    python3 plot.py path/to/run.csv  # custom CSV → path/to/run.png
"""

import sys
from collections import defaultdict
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

# Max number of satellites to show in the per-sat trajectory grid.
# Beyond this, only the overlay timelines (SMA, altitude, phase) are drawn.
GRID_MAX_SATS = 8

HERE = Path(__file__).parent
EARTH_RADIUS_KM = 6378.137
MU_KM3_S2 = 398600.4418


def _load_multisat_csv(path: Path) -> dict[str, np.ndarray]:
    """Return {sat_id: array of shape (N, 8) with cols t,x,y,z,vx,vy,vz,sma}.

    Handles both formats:
    - multi-sat: lines start with `sat-N,t,x,y,z,vx,vy,vz,sma,...`
    - single-sat: lines start with `t,x,y,z,vx,vy,vz,sma,...` (no sat_id column)
    """
    buckets: dict[str, list[list[float]]] = defaultdict(list)
    with path.open() as f:
        for line in f:
            if line.startswith("#") or not line.strip() or line[0].isalpha() and not line.startswith("sat-"):
                continue
            parts = line.rstrip().split(",")
            if parts[0].startswith("sat-"):
                sid = parts[0]
                data = parts[1:9]
            else:
                # Single-sat format: no sat_id column.
                sid = "sat-0"
                data = parts[0:8]
            try:
                buckets[sid].append([float(x) for x in data])
            except ValueError:
                continue
    return {sid: np.asarray(rows) for sid, rows in buckets.items()}


def _infer_burning(sma: np.ndarray) -> np.ndarray:
    """Heuristic: SMA changing > 0.1 km per step ≈ thrust firing.

    The WASM plugin doesn't expose throttle directly in the output, but SMA
    monotonically rises during burns and holds flat during Parked/Coast/Trim.
    """
    d = np.abs(np.diff(sma, prepend=sma[0]))
    return d > 0.1


def _target_circle(r_km: float, color: str, label: str | None = None, ls: str = ":"):
    theta = np.linspace(0, 2 * np.pi, 200)
    return r_km * np.cos(theta), r_km * np.sin(theta), color, ls, label


def main() -> None:
    if len(sys.argv) > 1:
        csv = Path(sys.argv[1]).resolve()
    else:
        csv = HERE / "phasing.csv"
    if not csv.exists():
        rrd = csv.with_suffix(".rrd")
        if rrd.exists():
            print(f"No {csv.name}. Run: orts convert --format csv {rrd} --output {csv}")
        else:
            print(f"No CSV at {csv}. Run:")
            print("  orts run --config orts.toml --output phasing.rrd")
            print(f"  orts convert --format csv phasing.rrd --output {csv.name}")
        return

    sats = _load_multisat_csv(csv)
    if not sats:
        print(f"No sat-* rows in {csv}")
        return
    # Natural sort so sat-10 comes after sat-2, not after sat-1.
    sat_ids = sorted(sats, key=lambda s: int(s.split("-")[-1]))
    n_sats = len(sat_ids)
    print(f"Loaded {n_sats} satellites: {', '.join(sat_ids[:6])}{' …' if n_sats > 6 else ''}")

    # tab10 is perceptually distinct; cycle for N > 10. Works for any N.
    cmap = plt.cm.tab10
    colors = [cmap(i % 10) for i in range(n_sats)]

    # Only lay out the per-sat trajectory grid for small constellations.
    show_grid = n_sats <= GRID_MAX_SATS
    r_init = EARTH_RADIUS_KM + 350.0
    r_op = EARTH_RADIUS_KM + 550.0
    theta = np.linspace(0, 2 * np.pi, 200)

    if show_grid:
        # Grid columns: 2 for n ≤ 4, else 4. Rows adapt to n_sats.
        grid_cols = 2 if n_sats <= 4 else 4
        grid_rows = (n_sats + grid_cols - 1) // grid_cols
        # Right half hosts overlay timelines; left half the per-sat grid.
        fig = plt.figure(figsize=(14, max(10, 2.5 * grid_rows + 3)))
        total_rows = max(3, grid_rows + 1)
        gs = fig.add_gridspec(
            total_rows, grid_cols * 2,
            height_ratios=[1.0] * grid_rows + [1.0] * (total_rows - grid_rows),
            hspace=0.4, wspace=0.35,
        )
    else:
        fig = plt.figure(figsize=(12, 10))
        gs = fig.add_gridspec(3, 1, height_ratios=[1.0, 1.0, 1.0], hspace=0.35)

    # Axes placement for overlay timelines.
    if show_grid:
        ax_sma = fig.add_subplot(gs[0, grid_cols:])
        ax_alt = fig.add_subplot(gs[1, grid_cols:], sharex=ax_sma) if grid_rows >= 2 \
            else fig.add_subplot(gs[-2, :], sharex=ax_sma)
        ax_phase = fig.add_subplot(gs[-1, :])
    else:
        ax_sma = fig.add_subplot(gs[0])
        ax_alt = fig.add_subplot(gs[1], sharex=ax_sma)
        ax_phase = fig.add_subplot(gs[2], sharex=ax_sma)

    # Per-sat trajectory grid (only when show_grid).
    for idx, sid in enumerate(sat_ids if show_grid else []):
        r = idx // grid_cols
        c = idx % grid_cols
        ax = fig.add_subplot(gs[r, c])
        d = sats[sid]
        t = d[:, 0]
        pos = d[:, 1:4]
        sma = d[:, 7]
        burning = _infer_burning(sma)

        # Earth
        earth = plt.Circle(
            (0, 0), EARTH_RADIUS_KM, color="#4477AA", alpha=0.5, zorder=0
        )
        ax.add_patch(earth)
        # Parking + operational circles
        ax.plot(
            r_init * np.cos(theta), r_init * np.sin(theta),
            "--", color="gray", lw=0.6, alpha=0.5, zorder=1,
        )
        ax.plot(
            r_op * np.cos(theta), r_op * np.sin(theta),
            ":", color="gray", lw=0.8, alpha=0.5, zorder=1,
        )
        # Coast (not burning)
        coast = ~burning
        ax.plot(
            np.where(coast, pos[:, 0], np.nan),
            np.where(coast, pos[:, 1], np.nan),
            color=colors[idx], lw=1.0, alpha=0.8, zorder=2,
        )
        # Burn (thrust firing)
        ax.plot(
            np.where(burning, pos[:, 0], np.nan),
            np.where(burning, pos[:, 1], np.nan),
            color="C3", lw=2.5, zorder=3, label="burn",
        )
        # Start / end markers
        ax.plot(pos[0, 0], pos[0, 1], "o", color="C2", markersize=5, zorder=4)
        ax.plot(pos[-1, 0], pos[-1, 1], "s", color=colors[idx], markersize=6, zorder=4)
        ax.set_aspect("equal")
        # Trim axis range around operational orbit
        lim = r_op + 500
        ax.set_xlim(-lim, lim)
        ax.set_ylim(-lim, lim)
        ax.set_xlabel("X [km]")
        ax.set_ylabel("Y [km]")
        # Title with delay
        # read raise_delay_s inferred from first row where SMA changes
        dsma = np.diff(sma)
        first_burn_idx = int(np.argmax(dsma > 0.1)) if (dsma > 0.1).any() else 0
        delay = t[first_burn_idx] if first_burn_idx > 0 else 0.0
        ax.set_title(f"{sid} (raise_delay = {delay:.0f} s)", fontsize=10)
        ax.grid(True, alpha=0.2)

    # --- SMA vs time overlay -------------------------------------------------
    for idx, sid in enumerate(sat_ids):
        d = sats[sid]
        ax_sma.plot(d[:, 0], d[:, 7], color=colors[idx], lw=1.2, label=sid)
    ax_sma.axhline(r_init, color="gray", ls="--", lw=0.6, alpha=0.6)
    ax_sma.axhline(r_op, color="gray", ls=":", lw=0.8, alpha=0.6)
    ax_sma.set_xlabel("Time [s]")
    ax_sma.set_ylabel("SMA [km]")
    ax_sma.set_title("SMA vs time — Parked (flat) → Hohmann burns step up to 6928 km")
    if n_sats <= 10:
        ax_sma.legend(loc="lower right", fontsize=8)
    ax_sma.grid(True, alpha=0.3)

    # --- Altitude vs time overlay --------------------------------------------
    for idx, sid in enumerate(sat_ids):
        d = sats[sid]
        alt = np.linalg.norm(d[:, 1:4], axis=1) - EARTH_RADIUS_KM
        ax_alt.plot(d[:, 0], alt, color=colors[idx], lw=1.0, alpha=0.85, label=sid)
    ax_alt.axhline(350, color="gray", ls="--", lw=0.6, alpha=0.6)
    ax_alt.axhline(550, color="gray", ls=":", lw=0.8, alpha=0.6)
    ax_alt.set_xlabel("Time [s]")
    ax_alt.set_ylabel("Altitude |r| − R⊕ [km]")
    ax_alt.set_title("Altitude vs time — Parked sats sit at 350 km, others climb")
    if n_sats <= 10:
        ax_alt.legend(loc="lower right", fontsize=8)
    ax_alt.grid(True, alpha=0.3)

    # --- Phase angle vs time -------------------------------------------------
    # True longitude (argument of latitude for equatorial orbit) = atan2(y, x)
    # but drifts by 2π each orbit. To visualize phasing clearly, compute the
    # angle relative to sat-0 (or the first sat) at the same time.
    # Interpolate all sats onto a common time grid.
    t_all = sats[sat_ids[0]][:, 0]
    ref = sats[sat_ids[0]]
    ref_angle = np.unwrap(np.arctan2(ref[:, 2], ref[:, 1]))
    for idx, sid in enumerate(sat_ids):
        d = sats[sid]
        # Raw atan2 difference → per-sample wrap to [-π, π] → unwrap the
        # sequence. Tracks continuous phase evolution so sat-3 shows a
        # monotonic 0 → +270° drift instead of jumping at ±180°.
        ang_raw = np.arctan2(d[:, 2], d[:, 1])
        ref_raw = np.arctan2(sats[sat_ids[0]][:, 2], sats[sat_ids[0]][:, 1])
        n = min(len(ang_raw), len(ref_raw))
        diff = ang_raw[:n] - ref_raw[:n]
        diff_wrapped = np.mod(diff + np.pi, 2 * np.pi) - np.pi
        rel_deg = np.degrees(np.unwrap(diff_wrapped))
        ax_phase.plot(t_all[:n], rel_deg, color=colors[idx], lw=1.3, label=sid)

    # Theoretical target offsets.
    for target in (0, 90, 180, 270):
        ax_phase.axhline(target, color="gray", ls=":", lw=0.5, alpha=0.5)

    ax_phase.set_xlabel("Time [s]")
    ax_phase.set_ylabel(r"Phase relative to sat-0 $\Delta\varphi$ [deg]")
    ax_phase.set_title(
        r"In-plane phase vs time — differential drift during Parked, frozen after Hohmann"
    )
    if n_sats <= 10:
        ax_phase.legend(loc="center right", fontsize=9)
    ax_phase.grid(True, alpha=0.3)
    # Auto-extend phase y-range if any sat exceeds 270° (e.g. N > 4 constellations).
    all_phases = []
    ref_raw = np.arctan2(sats[sat_ids[0]][:, 2], sats[sat_ids[0]][:, 1])
    for sid in sat_ids:
        d = sats[sid]
        ang_raw = np.arctan2(d[:, 2], d[:, 1])
        n = min(len(ang_raw), len(ref_raw))
        diff = ang_raw[:n] - ref_raw[:n]
        diff_wrapped = np.mod(diff + np.pi, 2 * np.pi) - np.pi
        all_phases.append(np.degrees(np.unwrap(diff_wrapped)))
    ymax = max(max(np.max(p), 270) for p in all_phases) + 40
    ymin = min(min(np.min(p), 0) for p in all_phases) - 40
    ax_phase.set_ylim(ymin, ymax)

    fig.suptitle(
        f"Constellation Phasing ({n_sats} × 350→550 km Hohmann)",
        y=0.995, fontsize=13,
    )
    out = csv.with_suffix(".png") if csv.name != "phasing.csv" else HERE / "constellation_phasing.png"
    fig.savefig(out, dpi=140, bbox_inches="tight")
    print(f"Saved {out}")
    plt.close(fig)


if __name__ == "__main__":
    main()
