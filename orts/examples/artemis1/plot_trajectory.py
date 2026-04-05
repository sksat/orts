# /// script
# requires-python = ">=3.10"
# dependencies = ["matplotlib>=3.8", "numpy>=1.24"]
# ///
"""Artemis 1 — 2D trajectory + error plots.

Usage:
    uv run orts/examples/artemis1/plot_trajectory.py

Reads `artemis1.rrd` (emitted by `cargo run --example artemis1 -p orts
--features fetch-horizons`) via the `orts convert` CLI and produces
four PNGs alongside this script:

- `artemis1_full_mission.png` — all three recorded phases (outbound
  coast, DRI→DRDI chain, return coast) overlaid in a single panel
  grid. Use this to see the end-to-end mission shape at a glance.
- `artemis1_outbound.png` — outbound coast phase zoom.
- `artemis1_chain.png` — DRI→DRDI chain zoom (includes the DRO
  retrograde loop and the two burn markers).
- `artemis1_return.png` — return coast phase zoom.

Each PNG uses the same 2×2 layout: ECI XY, Moon-centered XY, position
error magnitude over time (log scale), position error x/y/z
components. The per-phase PNGs restrict the data to that phase's sim
time window; the full-mission PNG shows all three phases on a single
timeline with unlogged gaps between them.

The Python script is a thin visualization wrapper; all physics (force
models, propagation, Method B burn verification) happens in the Rust
example. The CSV conversion path (`orts convert`) is the same one the
apollo11 example uses so the loader style matches.
"""
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

R_EARTH = 6378.137  # km
R_MOON = 1737.4  # km

SCRIPT_DIR = Path(__file__).parent
RRD_PATH = SCRIPT_DIR / "artemis1.rrd"

# Mission epoch = sim_time = 0 reference. Must match
# `MISSION_EPOCH_ISO` in main.rs so the per-phase slicing below lands
# on the correct sim_time ranges.
MISSION_EPOCH_ISO = "2022-11-17T00:00:00Z"

# Burn midpoints used for annotation. Must match the `MANEUVERS`
# constant in main.rs — update here whenever MANEUVERS is regenerated
# from extract_burns.py.
DRI_MID_ISO = "2022-11-25T21:52:45Z"
DRDI_MID_ISO = "2022-12-01T21:53:00Z"

# Unmodelled powered flyby epochs (from NASA Artemis 1 mission
# profile). The force model does not carry these burns, so fill-phase
# propagations will visibly diverge from the Horizons reference
# shortly after these epochs. Used only for annotation on the
# full-mission error plot.
OPF_EPOCH_ISO = "2022-11-21T12:44:00Z"
RPF_EPOCH_ISO = "2022-12-05T16:43:00Z"


@dataclass
class Phase:
    """One recorded mission segment.

    `key` is the short identifier used in output file names,
    `title` is the human-readable label shown in plot headers, and
    `start_iso` / `end_iso` bound the phase on the mission-elapsed-time
    axis.
    """

    key: str
    title: str
    start_iso: str
    end_iso: str


# Recorded phase list — must stay in sync with the phase sequence in
# main.rs. The three "verified" phases (outbound / chain / return)
# each start from a fresh Horizons state for independent error
# budgeting; the two "fill" phases stitch the mission into one
# continuous ~23-day propagation, and carry unmodelled powered
# flybys (OPF / RPF) so they visibly diverge from Horizons inside the
# gap interval — a feature, not a bug, since that divergence is
# exactly why we verify the three segments in isolation.
PHASES = [
    Phase("outbound", "Outbound (trans-lunar, 3 d)", "2022-11-17T00:00:00Z", "2022-11-20T00:00:00Z"),
    Phase("chain", "DRI → DRDI chain (6 d)", "2022-11-25T21:40:00Z", "2022-12-01T22:06:00Z"),
    Phase("return", "Return (trans-Earth, 4 d)", "2022-12-06T00:00:00Z", "2022-12-10T00:00:00Z"),
]

# Fill phases — recorded but not individually plotted. Useful if you
# ever want to slice the full-mission data down to just the fill
# intervals for diagnostic purposes.
FILL_PHASES = [
    Phase("opf_fill", "Outbound → chain fill (OPF inside)", "2022-11-20T00:00:00Z", "2022-11-25T21:40:00Z"),
    Phase("rpf_fill", "Chain → return fill (RPF inside)", "2022-12-01T22:06:00Z", "2022-12-06T00:00:00Z"),
]


def parse_iso_utc(s: str) -> datetime:
    """Parse an ISO 8601 `Z` timestamp to a naive UTC datetime."""
    return datetime.strptime(s.rstrip("Z"), "%Y-%m-%dT%H:%M:%S")


def sec_since_mission_epoch(iso: str) -> float:
    mission = parse_iso_utc(MISSION_EPOCH_ISO)
    return (parse_iso_utc(iso) - mission).total_seconds()


def load_entities() -> dict[str, np.ndarray]:
    """Run `orts convert` and split rows by entity path.

    Returns a dict keyed by entity path with values being `(n, 7)`
    arrays: `(t, x, y, z, vx, vy, vz)`. The `t` column is mission
    elapsed time in seconds (sim_time in the RRD, origin at
    `MISSION_EPOCH_ISO`).

    Each mission phase logs into its own entity subtree
    (`/world/sat/artemis1/<phase_key>`, etc.), so this function
    returns ~20 entity paths for a 5-phase recording. Use
    [`entities_for_phase`] below to select the four entities belonging
    to a single phase.
    """
    workspace_root = SCRIPT_DIR.parents[2]  # orts/examples/artemis1 → orts
    if not RRD_PATH.exists():
        sys.exit(
            f"error: {RRD_PATH} does not exist.\n"
            f"       run `cargo run --release --example artemis1 -p orts \\\n"
            f"              --features fetch-horizons` first to generate it."
        )

    result = subprocess.run(
        [
            "cargo", "run", "--bin", "orts", "-q", "--",
            "convert", str(RRD_PATH), "--format", "csv",
        ],
        capture_output=True, text=True, timeout=180,
        cwd=str(workspace_root),
    )
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        sys.exit(result.returncode)

    rows: dict[str, list[tuple[float, ...]]] = {}
    for line in result.stdout.strip().split("\n"):
        if line.startswith("#") or not line:
            continue
        p = line.split(",")
        sid = p[0]
        vals = tuple(float(x) for x in p[1:])
        rows.setdefault(sid, []).append(vals)

    return {sid: np.array(v) for sid, v in rows.items()}


# Canonical entity sub-paths used by the four logged families.
# Plot code accesses these via `entities_for_phase(phase_key)` which
# substitutes the phase key into the last path segment.
ENTITY_KINDS = {
    "sat": "/world/sat/artemis1/{phase_key}",
    "ref": "/world/ref/artemis1/{phase_key}",
    "err": "/world/analysis/error_km/{phase_key}",
    "moon": "/world/moon/{phase_key}",
}


def entities_for_phase(
    entities: dict[str, np.ndarray], phase_key: str
) -> dict[str, np.ndarray]:
    """Select the four entity arrays belonging to `phase_key`.

    Returns a dict with the OLD entity-kind keys (`/world/sat/...`,
    `/world/ref/...`, `/world/analysis/error_km`, `/world/moon`) so
    `plot_panels` can remain phase-agnostic — it just looks up the
    four canonical paths it expects.

    If a phase has no samples (e.g., the chain phase is skipped when
    `BURN_CHAIN_INDICES.len() < 2`), returns empty arrays for each
    kind so the downstream plot code can gracefully skip panels.
    """
    out: dict[str, np.ndarray] = {}
    for kind, template in ENTITY_KINDS.items():
        path = template.format(phase_key=phase_key)
        arr = entities.get(path)
        if arr is None:
            arr = np.empty((0, 7))
        # Re-key under the canonical plot-code path so the rest of
        # `plot_panels` doesn't need to know about the phase suffix.
        canonical = template.format(phase_key="").rstrip("/")
        # `plot_panels` reads `/world/sat/artemis1`, `/world/ref/artemis1`,
        # `/world/analysis/error_km`, `/world/moon` — rstrip leaves
        # exactly those strings.
        out[canonical] = arr
    return out


def concat_phases(
    entities: dict[str, np.ndarray], phase_keys: list[str]
) -> dict[str, np.ndarray]:
    """Concatenate the per-phase entity arrays for the full-mission
    plot.

    For each canonical entity kind (sat / ref / err / moon), stack
    the phase arrays in the given order and then sort by sim_time
    (the phases may already be sim_time-ordered, but stacking is
    order-independent up to sort).
    """
    combined: dict[str, list[np.ndarray]] = {
        template.format(phase_key="").rstrip("/"): [] for template in ENTITY_KINDS.values()
    }
    for key in phase_keys:
        phase_ents = entities_for_phase(entities, key)
        for canonical, arr in phase_ents.items():
            if len(arr) > 0:
                combined[canonical].append(arr)
    out: dict[str, np.ndarray] = {}
    for canonical, arr_list in combined.items():
        if arr_list:
            stacked = np.vstack(arr_list)
            stacked = stacked[np.argsort(stacked[:, 0], kind="stable")]
            out[canonical] = stacked
        else:
            out[canonical] = np.empty((0, 7))
    return out


def break_on_gaps(arr: np.ndarray, gap_threshold_s: float = 600.0) -> np.ndarray:
    """Insert a NaN row wherever adjacent timestamps are separated
    by more than `gap_threshold_s` seconds.

    Rationale: the full-mission plot concatenates three recorded
    phases (outbound, chain, return) on one sim_time axis with
    ~5-day unlogged gaps between them. Without this preprocessing
    matplotlib draws a straight line across each gap, connecting e.g.
    the end of outbound directly to the start of the chain at DRI
    pre and creating a spurious "linear jump" in every trajectory
    panel. A NaN row breaks the line at the gap without altering the
    numeric data of the logged samples.
    """
    if len(arr) < 2:
        return arr.copy()
    dt = np.diff(arr[:, 0])
    gap_idx = np.where(dt > gap_threshold_s)[0]
    if len(gap_idx) == 0:
        return arr.copy()
    parts: list[np.ndarray] = []
    start = 0
    for g in gap_idx:
        parts.append(arr[start : g + 1])
        nan_row = np.full((1, arr.shape[1]), np.nan)
        parts.append(nan_row)
        start = g + 1
    parts.append(arr[start:])
    return np.vstack(parts)


def break_all(entities: dict[str, np.ndarray]) -> dict[str, np.ndarray]:
    return {k: break_on_gaps(v) for k, v in entities.items()}


def eci_to_rotating(
    pos_eci: np.ndarray, moon_pos: np.ndarray, moon_vel: np.ndarray
) -> np.ndarray:
    """Transform a stack of ECI positions to the Earth-Moon rotating
    frame.

    The rotating frame has its origin at Earth (ECI origin) with the
    X axis pointing toward the Moon, Z axis along the Earth-Moon
    angular momentum (perpendicular to the orbital plane), and Y =
    Z × X completing the right-handed triad. In this frame both
    Earth and Moon are stationary: Earth sits at (0, 0) and the Moon
    sits at (|EM|, 0) ≈ (384,000, 0) km — which is the "Earth and
    Moon fixed" view the Artemis 1 DRO is conventionally visualized
    in.

    The Moon's instantaneous velocity is needed to define the Z axis
    (via `position × velocity`); we use the `moon_vel` column that
    `record_chain_trajectory` / `record_coast_phase` log directly
    (central-differenced from the Horizons ephemeris on the Rust
    side), which sidesteps the need for a numerical gradient and
    avoids artefacts at phase boundaries.

    Inputs are `(N, 3)` arrays in km and km/s. Returns an `(N, 3)`
    array in km (rotating-frame coordinates).
    """
    em_dist = np.linalg.norm(moon_pos, axis=1, keepdims=True)
    ex = moon_pos / em_dist
    ez = np.cross(moon_pos, moon_vel)
    ez = ez / np.linalg.norm(ez, axis=1, keepdims=True)
    ey = np.cross(ez, ex)
    rx = np.sum(pos_eci * ex, axis=1)
    ry = np.sum(pos_eci * ey, axis=1)
    rz = np.sum(pos_eci * ez, axis=1)
    return np.column_stack([rx, ry, rz])


def plot_panels(
    axes,
    entities: dict[str, np.ndarray],
    burn_markers: list[tuple[float, str, str]],
) -> None:
    """Draw the four standard panels into `axes` (flattened 2×2 grid).

    `burn_markers` is a list of `(mission_elapsed_hours, color, label)`
    annotations drawn on the two error panels as vertical lines.
    """
    sat = entities["/world/sat/artemis1"]
    ref = entities["/world/ref/artemis1"]
    moon = entities["/world/moon"]
    err = entities["/world/analysis/error_km"]

    t_s = sat[:, 0]  # mission elapsed seconds
    t_h = t_s / 3600.0

    err_pos = err[:, 1:4]
    err_mag = np.linalg.norm(err_pos, axis=1)

    # -------- Panel 0: ECI XY overview ----------------------------
    ax = axes[0]
    ax.plot(
        sat[:, 1] / 1000, sat[:, 2] / 1000,
        "b-", lw=0.9, alpha=0.85, label="Propagated (orts)",
    )
    ax.plot(
        ref[:, 1] / 1000, ref[:, 2] / 1000,
        "r--", lw=0.9, alpha=0.65, label="JPL Horizons (Orion −1023)",
    )
    ax.plot(
        moon[:, 1] / 1000, moon[:, 2] / 1000,
        color="#888", lw=0.6, alpha=0.6, label="Moon",
    )
    ax.plot(0, 0, "o", color="#4488ff", ms=10, label="Earth")
    ax.set_aspect("equal")
    ax.set_xlabel("X (×1000 km, ECI J2000)")
    ax.set_ylabel("Y (×1000 km, ECI J2000)")
    ax.set_title("ECI XY")
    ax.legend(fontsize=7, loc="best")
    ax.grid(True, alpha=0.3)

    # -------- Panel 1: Earth-Moon rotating frame -----------------
    #
    # The rotating frame has X pointing toward the Moon, Z along the
    # Earth-Moon angular momentum, and Y completing the right-handed
    # triad. In this frame both Earth (origin) and Moon (at x ≈
    # 384,000 km, y = 0) are stationary, so the DRO retrograde loop
    # shows up as a closed(-ish) curve around the Moon rather than
    # the inertial-frame spiral that ECI shows.
    #
    # NaN rows inserted by `break_on_gaps` produce `nan` entries in
    # the moon_pos / moon_vel arrays; `eci_to_rotating` propagates
    # the NaN through cleanly and matplotlib draws a gap, which is
    # the right behaviour for the full-mission plot.
    ax = axes[1]
    sat_rot = eci_to_rotating(sat[:, 1:4], moon[:, 1:4], moon[:, 4:7]) / 1000
    ref_rot = eci_to_rotating(ref[:, 1:4], moon[:, 1:4], moon[:, 4:7]) / 1000
    # Earth-Moon distance (for placing the Moon marker). Use a
    # nan-safe mean so the presence of gap rows does not poison the
    # reference distance.
    em_dist_mm = np.linalg.norm(moon[:, 1:4], axis=1) / 1000.0
    em_mean = float(np.nanmean(em_dist_mm))
    ax.plot(sat_rot[:, 0], sat_rot[:, 1], "b-", lw=0.9, alpha=0.85, label="Propagated")
    ax.plot(ref_rot[:, 0], ref_rot[:, 1], "r--", lw=0.9, alpha=0.65, label="Horizons")
    # Earth at the origin, Moon at mean Earth-Moon distance along +x.
    # Disc radii are drawn to scale (small blobs at the chain scale).
    theta = np.linspace(0, 2 * np.pi, 120)
    earth_r_mm = R_EARTH / 1000.0
    moon_r_mm = R_MOON / 1000.0
    ax.fill(earth_r_mm * np.cos(theta), earth_r_mm * np.sin(theta), color="#4488ff", alpha=0.8)
    ax.plot(0, 0, "o", color="#4488ff", ms=6, label="Earth")
    ax.fill(
        em_mean + moon_r_mm * np.cos(theta),
        moon_r_mm * np.sin(theta),
        color="#ccc",
        alpha=0.8,
    )
    ax.plot(em_mean, 0, "o", color="#888", ms=5, label="Moon")
    # First / last spacecraft points (nan-skipping).
    first_finite = np.argmax(np.isfinite(sat_rot[:, 0]))
    last_finite = len(sat_rot) - 1 - np.argmax(np.isfinite(sat_rot[::-1, 0]))
    ax.plot(sat_rot[first_finite, 0], sat_rot[first_finite, 1], "b^", ms=9, label="Start")
    ax.plot(sat_rot[last_finite, 0], sat_rot[last_finite, 1], "bv", ms=9, label="End")
    ax.set_aspect("equal")
    ax.set_xlabel("X (×1000 km, Earth-Moon rotating)")
    ax.set_ylabel("Y (×1000 km, Earth-Moon rotating)")
    ax.set_title("Earth-Moon rotating frame (Earth and Moon fixed)")
    ax.legend(fontsize=7, loc="best")
    ax.grid(True, alpha=0.3)

    # -------- Panel 2: position error magnitude (log) -------------
    #
    # The error grows by ~4 orders of magnitude across the full
    # mission (from ~0.1 km in the outbound phase to ~1000 km at
    # DRDI end). A linear y axis buries the early-phase dynamics; a
    # log y axis reveals the growth regime clearly.
    ax = axes[2]
    # Clip tiny values to avoid log(0) artifacts when the initial
    # sample is bit-identical to the Horizons reference (fetch_orion
    # at phase start returns the same sample the Horizons table
    # interpolates at `sim_time = 0` within each phase).
    err_mag_clipped = np.clip(err_mag, 1e-4, None)
    ax.semilogy(t_h, err_mag_clipped, "b-", lw=1.0, label="|propagated − reference|")
    for mark_h, color, label in burn_markers:
        ax.axvline(mark_h, color=color, ls=":", lw=1.3, alpha=0.8)
        ax.text(
            mark_h, ax.get_ylim()[1] * 0.6, f" {label}",
            color=color, fontsize=9, va="top",
        )
    ax.set_xlabel("Mission elapsed time (hours since 2022-11-17T00:00Z)")
    ax.set_ylabel("|Δ position| (km, log scale)")
    ax.set_title("Position error magnitude")
    ax.grid(True, alpha=0.3, which="both")
    ax.legend(fontsize=8, loc="lower right")

    # -------- Panel 3: position error components ------------------
    ax = axes[3]
    ax.plot(t_h, err_pos[:, 0], "r-", lw=0.9, label="Δx", alpha=0.85)
    ax.plot(t_h, err_pos[:, 1], "g-", lw=0.9, label="Δy", alpha=0.85)
    ax.plot(t_h, err_pos[:, 2], "b-", lw=0.9, label="Δz", alpha=0.85)
    ax.axhline(0, color="k", lw=0.5, alpha=0.5)
    for mark_h, color, _label in burn_markers:
        ax.axvline(mark_h, color=color, ls=":", lw=1.3, alpha=0.8)
    ax.set_xlabel("Mission elapsed time (hours since 2022-11-17T00:00Z)")
    ax.set_ylabel("Δ component (km, ECI J2000)")
    ax.set_title("Position error components")
    ax.legend(fontsize=8, loc="best")
    ax.grid(True, alpha=0.3)


def compute_burn_markers(include_flybys: bool = False) -> list[tuple[float, str, str]]:
    dri = sec_since_mission_epoch(DRI_MID_ISO) / 3600.0
    drdi = sec_since_mission_epoch(DRDI_MID_ISO) / 3600.0
    markers = [
        (dri, "#e07a00", "DRI"),
        (drdi, "#008a4a", "DRDI"),
    ]
    if include_flybys:
        opf = sec_since_mission_epoch(OPF_EPOCH_ISO) / 3600.0
        rpf = sec_since_mission_epoch(RPF_EPOCH_ISO) / 3600.0
        # Muted purple/magenta for flybys so they read as "unmodelled"
        # vs. the orange/green for modelled impulsive burns.
        markers.extend(
            [
                (opf, "#9b4dca", "OPF (unmodelled)"),
                (rpf, "#9b4dca", "RPF (unmodelled)"),
            ]
        )
    return markers


def plot_to_file(
    entities: dict[str, np.ndarray],
    title: str,
    subtitle: str,
    out_name: str,
    burn_markers: list[tuple[float, str, str]],
) -> None:
    fig, axes = plt.subplots(2, 2, figsize=(14, 11))
    plot_panels(list(axes.flat), entities, burn_markers)
    fig.suptitle(f"{title}\n{subtitle}", fontsize=13)
    plt.tight_layout()
    out = SCRIPT_DIR / out_name
    plt.savefig(out, dpi=150)
    print(f"Saved {out}")

    # Headline error summary for stdout. `nan`-safe because the
    # full-mission plot inserts NaN rows at phase gaps via
    # `break_on_gaps`; `np.nanmax` / `np.nanargmax` skip those.
    err = entities["/world/analysis/error_km"]
    if len(err) > 0:
        mag = np.linalg.norm(err[:, 1:4], axis=1)
        t_h = err[:, 0] / 3600.0
        finite_mask = np.isfinite(mag)
        if finite_mask.any():
            finite_mag = mag[finite_mask]
            finite_th = t_h[finite_mask]
            peak_i = int(np.argmax(finite_mag))
            final_i = -1
            print(
                f"    final position error:  {finite_mag[final_i]:9.3f} km  @ t = {finite_th[final_i]:7.2f} h\n"
                f"    peak  position error:  {finite_mag[peak_i]:9.3f} km  @ t = {finite_th[peak_i]:7.2f} h"
            )

    plt.close(fig)


def main() -> None:
    ents = load_entities()

    # Full mission plot — concatenate all five recorded phases in
    # chronological order. Each phase's samples live in its own
    # entity subtree (e.g. `/world/sat/artemis1/outbound`) so the
    # stitch happens at the Python layer instead of the RRD layer,
    # which avoids sim_time-boundary sample collisions that would
    # otherwise poison per-phase slicing. Unmodelled powered flybys
    # (OPF, RPF) are annotated on the error panel so the reader can
    # see exactly where the pure-coast fill phases diverge from
    # Horizons.
    full_keys = ["outbound", "opf_fill", "chain", "rpf_fill", "return"]
    full_ents = concat_phases(ents, full_keys)
    full_start = PHASES[0].start_iso
    full_end = PHASES[-1].end_iso
    plot_to_file(
        break_all(full_ents),
        "Artemis 1 full mission — orts vs JPL Horizons (Orion −1023)",
        f"{full_start}  →  {full_end}    (5 recorded phases: 3 verified + 2 gap fills)",
        "artemis1_full_mission.png",
        compute_burn_markers(include_flybys=True),
    )

    # Per-phase plots — one PNG per verified phase. Fill phases
    # (`FILL_PHASES`) are intentionally not plotted individually
    # because their divergence from Horizons is the whole point and
    # already visible on the full-mission plot above.
    burn_markers_phase = compute_burn_markers(include_flybys=False)
    for phase in PHASES:
        phase_ents = entities_for_phase(ents, phase.key)
        # Only annotate burn markers that actually fall within this
        # phase's window so the line labels do not bleed off-axis.
        t0_h = sec_since_mission_epoch(phase.start_iso) / 3600.0
        t1_h = sec_since_mission_epoch(phase.end_iso) / 3600.0
        markers_here = [(h, c, l) for (h, c, l) in burn_markers_phase if t0_h - 0.01 <= h <= t1_h + 0.01]
        plot_to_file(
            phase_ents,
            f"Artemis 1 — {phase.title}",
            f"{phase.start_iso}  →  {phase.end_iso}",
            f"artemis1_{phase.key}.png",
            markers_here,
        )


if __name__ == "__main__":
    main()
