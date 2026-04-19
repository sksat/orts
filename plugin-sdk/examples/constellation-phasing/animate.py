#!/usr/bin/env python3
"""Generate an animated GIF of the constellation-phasing deployment.

Shows 4 satellites in ECI XY plane drifting apart during their parked
phases and then climbing to operational altitude via Hohmann transfers.

    uv run animate.py  # or: python3 animate.py
    # -> constellation_phasing.gif
"""

import shutil
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.animation import FuncAnimation, PillowWriter

HERE = Path(__file__).parent
EARTH_RADIUS_KM = 6378.137
FPS = 15
N_FRAMES = 1000
TRAIL_SAMPLES = 200  # length of the trail behind each satellite
# When satellites are within this distance [km], they are treated as a
# visually overlapping cluster and their *display* position is spread out
# into a small ring so each dot stays individually visible. Actual data
# (trails, SMA title, phase plot) still uses true positions.
CLUSTER_THRESHOLD_KM = 250.0
CLUSTER_RING_RADIUS_KM = 180.0


def _spread_overlapping(xy: np.ndarray) -> np.ndarray:
    """Return a copy of `xy` (shape (n, 2)) where overlapping points are
    pushed out onto a small ring around their centroid.

    Two points are considered overlapping if they are within
    CLUSTER_THRESHOLD_KM. Transitive closure groups 3+ point clusters.
    """
    n = len(xy)
    parent = list(range(n))

    def find(i: int) -> int:
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    def union(i: int, j: int) -> None:
        pi, pj = find(i), find(j)
        if pi != pj:
            parent[pi] = pj

    for i in range(n):
        for j in range(i + 1, n):
            if np.linalg.norm(xy[i] - xy[j]) < CLUSTER_THRESHOLD_KM:
                union(i, j)

    clusters: dict[int, list[int]] = defaultdict(list)
    for i in range(n):
        clusters[find(i)].append(i)

    out = xy.copy()
    for members in clusters.values():
        if len(members) <= 1:
            continue
        members = sorted(members)
        center = np.mean([xy[i] for i in members], axis=0)
        # Assign each cluster member a fixed slot (deterministic so the
        # frame-to-frame animation is smooth: sat-0 stays at angle 0,
        # sat-1 at angle 2π/k, etc.).
        for slot, mi in enumerate(members):
            angle = 2 * np.pi * slot / len(members)
            out[mi] = center + CLUSTER_RING_RADIUS_KM * np.array(
                [np.cos(angle), np.sin(angle)]
            )
    return out


def _load_multisat_csv(path: Path) -> dict[str, np.ndarray]:
    """Return {sat_id: array of shape (N, 8) = t,x,y,z,vx,vy,vz,sma}.

    Handles single-sat (no leading sat-N column) and multi-sat formats.
    """
    buckets: dict[str, list[list[float]]] = defaultdict(list)
    with path.open() as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            if line[0].isalpha() and not line.startswith("sat-"):
                continue  # header like `t[s],x[km],...`
            parts = line.rstrip().split(",")
            if parts[0].startswith("sat-"):
                sid = parts[0]
                data = parts[1:9]
            else:
                sid = "sat-0"
                data = parts[0:8]
            try:
                buckets[sid].append([float(x) for x in data])
            except ValueError:
                continue
    return {sid: np.asarray(rows) for sid, rows in buckets.items()}


def main() -> None:
    # Minimal flag parsing: one optional positional CSV, and `--wide` to use
    # the horizontal layout (ECI on left, altitude + phase stacked on right).
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    wide = "--wide" in sys.argv[1:]
    no_compress = "--no-compress" in sys.argv[1:]
    if args:
        csv = Path(args[0]).resolve()
    else:
        csv = HERE / "phasing.csv"
    if not csv.exists():
        rrd = csv.with_suffix(".rrd")
        hint = f"orts convert --format csv {rrd} --output {csv}" if rrd.exists() else \
            f"orts run --config orts.toml --output {rrd.name} && " \
            f"orts convert --format csv {rrd.name} --output {csv.name}"
        print(f"No {csv}. Run: {hint}")
        return

    sats = _load_multisat_csv(csv)
    if not sats:
        print(f"No sat-* rows in {csv}")
        return
    sat_ids = sorted(sats, key=lambda s: int(s.split("-")[-1]))

    # Common time axis — align to the shortest sat (should all be equal)
    n_rows = min(len(sats[sid]) for sid in sat_ids)
    t = sats[sat_ids[0]][:n_rows, 0]
    positions = {
        sid: sats[sid][:n_rows, 1:3]  # only ECI x, y
        for sid in sat_ids
    }
    sma = {sid: sats[sid][:n_rows, 7] for sid in sat_ids}
    altitude = {
        sid: np.linalg.norm(sats[sid][:n_rows, 1:4], axis=1) - EARTH_RADIUS_KM
        for sid in sat_ids
    }
    # Relative in-plane phase vs sat-0. Raw atan2 difference → per-sample
    # wrap to [-π, π] → unwrap the sequence, giving continuous evolution
    # from ~0 to each sat's target phase (sat-3 shows a monotonic 0 → 270°
    # drift instead of jumping at 180°). Small transient lags at t=0
    # display as small negative values (not bogus ~360° spikes).
    ref_raw = np.arctan2(positions[sat_ids[0]][:, 1], positions[sat_ids[0]][:, 0])
    rel_phase_deg = {}
    for sid in sat_ids:
        ang_raw = np.arctan2(positions[sid][:, 1], positions[sid][:, 0])
        diff = ang_raw - ref_raw
        diff_wrapped = np.mod(diff + np.pi, 2 * np.pi) - np.pi
        rel_phase_deg[sid] = np.degrees(np.unwrap(diff_wrapped))

    # Sub-sample to approximately N_FRAMES frames. Using round() instead of
    # floor-division keeps the frame count close to N_FRAMES even when
    # n_rows isn't a clean multiple (e.g. 1960 rows → step=2 → ~980 frames,
    # vs. step=1 → 1960 frames with integer division).
    step = max(1, round(n_rows / N_FRAMES))
    frames = list(range(0, n_rows, step))
    print(f"Animating {len(frames)} frames from {n_rows} rows (step={step}, fps={FPS})")

    # --- set up figure --------------------------------------------------------
    # Two layout modes:
    #   vertical (default): ECI on top, altitude + phase stacked below
    #   wide (--wide):      ECI on left (square), altitude + phase on right
    if wide:
        fig = plt.figure(figsize=(12, 6.5))
        gs = fig.add_gridspec(
            2, 2,
            width_ratios=[1.05, 1.0], height_ratios=[1.0, 1.0],
            wspace=0.28, hspace=0.3,
        )
        ax = fig.add_subplot(gs[:, 0])  # ECI spans both rows on the left
        ax_alt = fig.add_subplot(gs[0, 1])
        ax_phase = fig.add_subplot(gs[1, 1])
    else:
        fig = plt.figure(figsize=(6, 10.5))
        gs = fig.add_gridspec(3, 1, height_ratios=[1.4, 0.65, 0.65], hspace=0.38)
        ax = fig.add_subplot(gs[0])
        ax_alt = fig.add_subplot(gs[1])
        ax_phase = fig.add_subplot(gs[2])
    r_park = EARTH_RADIUS_KM + 350.0
    r_op = EARTH_RADIUS_KM + 550.0
    lim = r_op + 600

    # Static background
    earth = plt.Circle((0, 0), EARTH_RADIUS_KM, color="#4477AA", alpha=0.55, zorder=0)
    ax.add_patch(earth)
    theta = np.linspace(0, 2 * np.pi, 200)
    ax.plot(
        r_park * np.cos(theta), r_park * np.sin(theta),
        "--", color="gray", lw=0.7, alpha=0.5, label="parking 350 km",
    )
    ax.plot(
        r_op * np.cos(theta), r_op * np.sin(theta),
        ":", color="gray", lw=0.9, alpha=0.6, label="operational 550 km",
    )

    # tab10 colormap cycling for any N; trails still readable for N ≤ 10.
    cmap = plt.cm.tab10
    colors = [cmap(i % 10) for i in range(len(sat_ids))]
    trails = {}
    dots = {}
    alt_lines = {}
    alt_dots = {}
    phase_lines = {}
    phase_dots = {}
    for idx, sid in enumerate(sat_ids):
        (trail_line,) = ax.plot(
            [], [], "-", color=colors[idx], lw=1.6, alpha=0.8, zorder=2,
        )
        (dot,) = ax.plot(
            [], [], "o", color=colors[idx], markersize=9,
            markeredgecolor="white", markeredgewidth=1.0, zorder=3, label=sid,
        )
        trails[sid] = trail_line
        dots[sid] = dot
        # Altitude-timeline curves.
        (aline,) = ax_alt.plot([], [], "-", color=colors[idx], lw=1.2, alpha=0.9)
        (adot,) = ax_alt.plot(
            [], [], "o", color=colors[idx], markersize=5.5,
            markeredgecolor="white", markeredgewidth=0.8, zorder=3, label=sid,
        )
        alt_lines[sid] = aline
        alt_dots[sid] = adot
        # Phase-timeline curves.
        (pline,) = ax_phase.plot([], [], "-", color=colors[idx], lw=1.3, alpha=0.9)
        (pdot,) = ax_phase.plot(
            [], [], "o", color=colors[idx], markersize=5.5,
            markeredgecolor="white", markeredgewidth=0.8, zorder=3, label=sid,
        )
        phase_lines[sid] = pline
        phase_dots[sid] = pdot

    ax.set_aspect("equal")
    ax.set_xlim(-lim, lim)
    ax.set_ylim(-lim, lim)
    ax.set_xlabel("X [km]")
    ax.set_ylabel("Y [km]")
    if len(sat_ids) <= 10:
        ax.legend(loc="lower right", fontsize=9)
    ax.grid(True, alpha=0.2)
    title = ax.set_title("")

    # Altitude timeline axes.
    ax_alt.set_xlim(t[0], t[-1])
    ax_alt.set_ylim(320, 610)
    ax_alt.set_yticks([350, 450, 550])
    ax_alt.set_ylabel("Altitude [km]")
    ax_alt.axhline(350, color="gray", ls="--", lw=0.6, alpha=0.5)
    ax_alt.axhline(550, color="gray", ls=":", lw=0.8, alpha=0.5)
    ax_alt.grid(True, alpha=0.25)
    if len(sat_ids) <= 10:
        ax_alt.legend(loc="center right", fontsize=8, ncol=min(4, len(sat_ids)), framealpha=0.8)

    # Phase timeline axes. Auto-extend range if any sat exceeds 270°.
    phase_max = max(np.max(rel_phase_deg[sid]) for sid in sat_ids)
    phase_min = min(np.min(rel_phase_deg[sid]) for sid in sat_ids)
    ax_phase.set_xlim(t[0], t[-1])
    ax_phase.set_ylim(min(phase_min - 30, -30), max(phase_max + 40, 310))
    ax_phase.set_yticks([0, 90, 180, 270])
    ax_phase.set_xlabel("Time [s]")
    ax_phase.set_ylabel(r"$\Delta\varphi$ vs sat-0 [deg]")
    for target in (0, 90, 180, 270):
        ax_phase.axhline(target, color="gray", ls=":", lw=0.5, alpha=0.5)
    ax_phase.grid(True, alpha=0.25)

    def update(fi: int):
        i = frames[fi]
        artists = []
        # Trails use true positions (so the orbital paths are honest).
        for sid in sat_ids:
            xy = positions[sid]
            j0 = max(0, i - TRAIL_SAMPLES)
            trails[sid].set_data(xy[j0:i + 1, 0], xy[j0:i + 1, 1])
            artists.append(trails[sid])
        # Dots use spread-for-overlap positions so visually overlapping
        # satellites are still individually visible (Parked clusters).
        true_xy = np.array([positions[sid][i] for sid in sat_ids])
        shown_xy = _spread_overlapping(true_xy)
        for idx, sid in enumerate(sat_ids):
            dots[sid].set_data([shown_xy[idx, 0]], [shown_xy[idx, 1]])
            artists.append(dots[sid])
        # Altitude & phase timelines grow up to current frame.
        for sid in sat_ids:
            alt_lines[sid].set_data(t[: i + 1], altitude[sid][: i + 1])
            alt_dots[sid].set_data([t[i]], [altitude[sid][i]])
            artists.extend([alt_lines[sid], alt_dots[sid]])
            phase_lines[sid].set_data(t[: i + 1], rel_phase_deg[sid][: i + 1])
            phase_dots[sid].set_data([t[i]], [rel_phase_deg[sid][i]])
            artists.extend([phase_lines[sid], phase_dots[sid]])
        # Title: always show the clock; show per-sat Δφ only for small N,
        # wrapped onto multiple lines when > 4.
        t_s = t[i]
        header = f"t = {t_s:.0f} s  (≈ {t_s / 3600:5.2f} h)"
        if len(sat_ids) <= 12:
            per_line = 4 if len(sat_ids) <= 4 else 2
            entries = [
                f"{sid}: Δφ={rel_phase_deg[sid][i]:5.1f}°" for sid in sat_ids
            ]
            lines = [
                "  ".join(entries[k : k + per_line])
                for k in range(0, len(entries), per_line)
            ]
            title.set_text(header + "\n" + "\n".join(lines))
        else:
            title.set_text(header)
        artists.append(title)
        return artists

    anim = FuncAnimation(
        fig, update, frames=len(frames), interval=1000 / FPS, blit=False,
    )

    suffix = "_wide.gif" if wide else ".gif"
    if csv.name != "phasing.csv":
        out = csv.with_name(csv.stem + suffix)
    else:
        out = HERE / f"constellation_phasing{'_wide' if wide else ''}.gif"
    print(f"Rendering → {out}")
    anim.save(out, writer=PillowWriter(fps=FPS))
    plt.close(fig)

    raw_size = out.stat().st_size
    if no_compress:
        print(f"Saved {out}  ({raw_size / 1024 / 1024:.2f} MB, no compression)")
    else:
        _optimize_gif(out)
        final_size = out.stat().st_size
        print(
            f"Saved {out}  "
            f"({raw_size / 1024 / 1024:.2f} MB → {final_size / 1024 / 1024:.2f} MB)"
        )


def _optimize_gif(path: Path) -> None:
    """Palette-optimize the GIF in place using ffmpeg (no-op if missing).

    Reduces file size by ~4x by building a per-file palette (64 colors) and
    downscaling to 480 px wide. Preserves animation length and FPS.
    """
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        print("(ffmpeg not found, skipping palette optimization)")
        return

    with tempfile.TemporaryDirectory() as tmpdir:
        palette = Path(tmpdir) / "palette.png"
        optimized = Path(tmpdir) / "out.gif"
        # 1. Generate per-file palette.
        subprocess.run(
            [
                ffmpeg, "-y", "-loglevel", "error",
                "-i", str(path),
                "-vf", "scale=480:-1:flags=lanczos,palettegen=max_colors=64",
                str(palette),
            ],
            check=True,
        )
        # 2. Re-encode with that palette, downscaled.
        subprocess.run(
            [
                ffmpeg, "-y", "-loglevel", "error",
                "-i", str(path),
                "-i", str(palette),
                "-filter_complex",
                "scale=480:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=none",
                str(optimized),
            ],
            check=True,
        )
        shutil.move(str(optimized), str(path))


if __name__ == "__main__":
    main()
