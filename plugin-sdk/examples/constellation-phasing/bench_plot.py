#!/usr/bin/env python3
"""Plot hyperfine benchmark results produced by `bench.sh`.

Reads one or more `/tmp/orts-bench/result_<backend>_<mode>.json` files (hyperfine
JSON output) and writes `bench.png` with:

1. Wall-clock vs N (log-log)
2. Realtime factor (sim_duration / wall_clock) vs N
3. Cost [μs / sat / sim-s] vs N — shows scaling efficiency

Usage:
    python3 bench_plot.py                                # auto-find all json in /tmp/orts-bench
    python3 bench_plot.py /tmp/orts-bench/result_auto_quick.json  # single run
    python3 bench_plot.py run1.json run2.json            # compare multiple runs
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

HERE = Path(__file__).parent
DEFAULT_BENCH_DIR = Path("/tmp/orts-bench")

# Must match the constants in gen_bench_config.py — used to derive sim duration.
SIM_DURATION_S = 3000.0


def _load_hyperfine_json(path: Path) -> list[tuple[int, float, float]]:
    """Return list of (N, mean_seconds, stddev_seconds) from a hyperfine JSON.

    hyperfine `--export-json` writes a { "results": [{"command": str, "mean": f,
    "stddev": f, ...}, ...] } structure. We extract N from the command string
    which contains `bench_N<n>.toml`.
    """
    data = json.loads(path.read_text())
    out: list[tuple[int, float, float]] = []
    for r in data.get("results", []):
        cmd = r.get("command", "")
        m = re.search(r"bench_N(\d+)\.toml", cmd)
        if not m:
            continue
        n = int(m.group(1))
        mean = float(r["mean"])
        stddev = float(r.get("stddev", 0.0))
        out.append((n, mean, stddev))
    out.sort(key=lambda x: x[0])
    return out


def _label_from_path(p: Path) -> str:
    """Strip the hyperfine filename down to `<backend> <mode>`."""
    stem = p.stem  # result_auto_quick
    m = re.match(r"result_(\w+?)_(\w+)", stem)
    if m:
        return f"{m.group(1)} / {m.group(2)}"
    return stem


def main() -> None:
    if len(sys.argv) > 1:
        paths = [Path(a).resolve() for a in sys.argv[1:]]
    else:
        paths = sorted(DEFAULT_BENCH_DIR.glob("result_*.json"))
    if not paths:
        print(f"No hyperfine JSON found. Run `./bench.sh <backend> <mode>` first,")
        print(f"then look in {DEFAULT_BENCH_DIR} for result_*.json.")
        return

    series: dict[str, list[tuple[int, float, float]]] = {}
    for p in paths:
        if not p.exists():
            print(f"warn: missing {p}", file=sys.stderr)
            continue
        results = _load_hyperfine_json(p)
        if not results:
            print(f"warn: no results parsed from {p}", file=sys.stderr)
            continue
        series[_label_from_path(p)] = results
        print(f"Loaded {p.name}: {len(results)} points (N={[r[0] for r in results]})")

    if not series:
        print("No usable data.")
        return

    fig, (ax_wall, ax_rt, ax_cost) = plt.subplots(1, 3, figsize=(15, 4.8))
    colors = plt.cm.tab10

    for idx, (label, pts) in enumerate(series.items()):
        ns = np.array([p[0] for p in pts], dtype=float)
        means = np.array([p[1] for p in pts])
        stds = np.array([p[2] for p in pts])
        color = colors(idx % 10)

        # Wall-clock
        ax_wall.errorbar(ns, means, yerr=stds, marker="o", lw=1.5,
                         capsize=3, color=color, label=label)
        # Realtime factor = sim_duration / wall_clock
        rtf = SIM_DURATION_S / means
        ax_rt.plot(ns, rtf, marker="o", lw=1.5, color=color, label=label)
        # Cost per sat per sim-second (μs)
        cost = means * 1e6 / (ns * SIM_DURATION_S)
        cost_err = stds * 1e6 / (ns * SIM_DURATION_S)
        ax_cost.errorbar(ns, cost, yerr=cost_err, marker="o", lw=1.5,
                         capsize=3, color=color, label=label)

    # Reference O(N) linear dashed line (wall-clock) anchored at smallest N.
    first_label = next(iter(series))
    first_pts = series[first_label]
    ns_ref = np.array([p[0] for p in first_pts], dtype=float)
    ns_ref = np.array([ns_ref[0], ns_ref[-1]])
    mean_ref = np.array([p[1] for p in first_pts])
    linear = mean_ref[0] * ns_ref / ns_ref[0]
    ax_wall.plot(ns_ref, linear, "k:", lw=1.0, alpha=0.5, label=r"ideal $\propto N$")

    ax_wall.set_xscale("log", base=2)
    ax_wall.set_yscale("log")
    ax_wall.set_xlabel("N (satellites)")
    ax_wall.set_ylabel("Wall-clock [s]")
    ax_wall.set_title(f"Wall-clock vs N (sim duration = {SIM_DURATION_S:.0f}s)")
    ax_wall.grid(True, which="both", alpha=0.3)
    ax_wall.legend(loc="upper left", fontsize=9)

    ax_rt.set_xscale("log", base=2)
    ax_rt.axhline(1.0, color="red", ls=":", lw=0.8, alpha=0.6, label="realtime")
    ax_rt.set_xlabel("N (satellites)")
    ax_rt.set_ylabel("Realtime factor  sim / wall")
    ax_rt.set_title("Realtime factor (>1 = faster than realtime)")
    ax_rt.grid(True, which="both", alpha=0.3)
    ax_rt.legend(loc="upper right", fontsize=9)

    ax_cost.set_xscale("log", base=2)
    ax_cost.set_xlabel("N (satellites)")
    ax_cost.set_ylabel("Cost [μs / sat / sim-s]")
    ax_cost.set_title("Per-sat cost (flat = linear scaling)")
    ax_cost.grid(True, which="both", alpha=0.3)
    ax_cost.legend(loc="upper left", fontsize=9)

    fig.suptitle(
        "constellation-phasing — multi-sat WASM plugin scaling",
        y=1.00, fontsize=12,
    )
    out = HERE / "bench.png"
    fig.savefig(out, dpi=130, bbox_inches="tight")
    print(f"Saved {out}")
    plt.close(fig)


if __name__ == "__main__":
    main()
