#!/usr/bin/env python3
"""Extract Artemis 1 burn events from JPL Horizons velocity discontinuities.

For each sample pair in a Horizons vector-table fetch, we compute the
velocity change rate |Δv|/Δt. During coast phases this is dominated by
gravitational acceleration (O(1e-3 m/s²) near the Moon, O(1e-5 m/s²) in
free space). During a propulsive burn the rate jumps by 2-4 orders of
magnitude — the OMS-E engine on the Orion ESM produces ~0.1 to 1 m/s²
depending on mass, while coast rarely exceeds 1e-3 m/s². Thresholding at
a few mm/s² comfortably separates burns from coast.

The tool does two passes:

1. **Coarse scan** (default 5-minute step) over the whole mission to find
   candidate burns. Contiguous high-rate samples are clustered into
   "events" and reported.
2. **Fine scan** (`--zoom`) with a 30-second or finer step over a tight
   window around a known burn midpoint to get precise start/end/Δv.

Output is both a human-readable table and a Rust snippet suitable for
copy-pasting into a `MANEUVERS` array (with the `--rust` flag).

## Examples

    # Full mission scan at 5-minute resolution (default)
    ./extract_burns.py

    # Zoom in on the DRI burn and print Rust literals
    ./extract_burns.py --zoom 2022-11-25T21:50:00Z --window-min 30 \
                       --step 30s --rust

    # Tighter threshold to catch OTC/RTC sub-m/s corrections
    ./extract_burns.py --threshold 0.0002

## Requirements

Python 3.8+ standard library only (no numpy, no requests).

## Caching

Responses are cached to `~/.cache/orts/horizons-py/<sha1>.csv` with a
7-day TTL so repeated runs are free.
"""

from __future__ import annotations

import argparse
import hashlib
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import List, Sequence, Tuple
from urllib.parse import urlencode
from urllib.request import Request, urlopen

# ---------------------------------------------------------------------------
# Horizons API
# ---------------------------------------------------------------------------

HORIZONS_API = "https://ssd.jpl.nasa.gov/api/horizons.api"
DEFAULT_TARGET = "-1023"  # Artemis 1 Orion
DEFAULT_CENTER = "500@399"  # Earth geocenter
CACHE_DIR = Path.home() / ".cache" / "orts" / "horizons-py"
CACHE_TTL_SECONDS = 7 * 24 * 60 * 60


def _cache_key(target: str, center: str, start: str, stop: str, step: str) -> str:
    h = hashlib.sha1()
    for part in (target, center, start, stop, step):
        h.update(part.encode("utf-8"))
        h.update(b"|")
    return h.hexdigest()[:16]


def fetch_horizons(
    target: str, center: str, start: str, stop: str, step: str
) -> str:
    """Fetch a Horizons vector-table response, caching to disk."""
    key = _cache_key(target, center, start, stop, step)
    cache_file = CACHE_DIR / f"{key}.csv"
    if cache_file.exists():
        age = time.time() - cache_file.stat().st_mtime
        if age < CACHE_TTL_SECONDS:
            return cache_file.read_text(encoding="utf-8")

    params = {
        "format": "text",
        "COMMAND": f"'{target}'",
        "OBJ_DATA": "NO",
        "MAKE_EPHEM": "YES",
        "EPHEM_TYPE": "VECTORS",
        "CENTER": f"'{center}'",
        "START_TIME": f"'{start}'",
        "STOP_TIME": f"'{stop}'",
        "STEP_SIZE": f"'{step}'",
        "VEC_TABLE": "2",
        "OUT_UNITS": "KM-S",
        "CSV_FORMAT": "YES",
        "REF_SYSTEM": "ICRF",
        "REF_PLANE": "FRAME",
        "TIME_TYPE": "TDB",
    }
    url = f"{HORIZONS_API}?{urlencode(params)}"
    print(
        f"  fetching Horizons: {target} {start} → {stop} step={step}",
        file=sys.stderr,
    )
    req = Request(url, headers={"User-Agent": "orts-burn-extractor/1.0"})
    with urlopen(req, timeout=60) as resp:
        body = resp.read().decode("utf-8")

    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    cache_file.write_text(body, encoding="utf-8")
    return body


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------


@dataclass
class Sample:
    jd: float  # JD TDB
    calendar: str
    x: float
    y: float
    z: float
    vx: float
    vy: float
    vz: float

    def position(self) -> Tuple[float, float, float]:
        return (self.x, self.y, self.z)

    def velocity(self) -> Tuple[float, float, float]:
        return (self.vx, self.vy, self.vz)


def parse_vectors(csv_text: str) -> List[Sample]:
    """Parse a Horizons vector-table response into a list of Samples."""
    lines = csv_text.splitlines()
    try:
        soe = next(i for i, line in enumerate(lines) if line.strip() == "$$SOE")
        eoe = next(i for i, line in enumerate(lines) if line.strip() == "$$EOE")
    except StopIteration as e:
        # Horizons returns a plain-text error page (no $$SOE) for common
        # failures like "target does not exist at requested epoch", invalid
        # parameters, etc. Surface the most informative line.
        meaningful = [
            line
            for line in lines
            if line.strip()
            and not line.startswith("API ")
            and not line.startswith("*")
        ]
        hint = "\n".join(meaningful[:10]) if meaningful else "\n".join(lines[:10])
        raise RuntimeError(
            f"Horizons response missing $$SOE/$$EOE markers — likely an error "
            f"from the API. Message:\n{hint}"
        ) from e

    samples: List[Sample] = []
    for line in lines[soe + 1 : eoe]:
        line = line.strip()
        if not line:
            continue
        parts = [p.strip() for p in line.split(",")]
        # JDTDB, Calendar, X, Y, Z, VX, VY, VZ [, trailing empty]
        if len(parts) < 8:
            raise RuntimeError(f"Short Horizons row: {line!r}")
        samples.append(
            Sample(
                jd=float(parts[0]),
                calendar=parts[1],
                x=float(parts[2]),
                y=float(parts[3]),
                z=float(parts[4]),
                vx=float(parts[5]),
                vy=float(parts[6]),
                vz=float(parts[7]),
            )
        )
    if not samples:
        raise RuntimeError("No samples between $$SOE and $$EOE")
    return samples


# ---------------------------------------------------------------------------
# Burn detection
# ---------------------------------------------------------------------------


@dataclass
class SamplePairRate:
    """Per-sample-pair velocity-change rate (one entry per adjacent pair)."""

    a: Sample
    b: Sample
    dt_seconds: float
    dv: Tuple[float, float, float]  # km/s
    dv_mag_ms: float  # m/s
    rate_m_per_s2: float  # m/s² (dv_mag / dt)


def compute_rates(samples: Sequence[Sample]) -> List[SamplePairRate]:
    out: List[SamplePairRate] = []
    for a, b in zip(samples, samples[1:]):
        dt = (b.jd - a.jd) * 86_400.0
        if dt <= 0:
            continue
        dvx = b.vx - a.vx
        dvy = b.vy - a.vy
        dvz = b.vz - a.vz
        dv_mag_km = (dvx * dvx + dvy * dvy + dvz * dvz) ** 0.5
        out.append(
            SamplePairRate(
                a=a,
                b=b,
                dt_seconds=dt,
                dv=(dvx, dvy, dvz),
                dv_mag_ms=dv_mag_km * 1000.0,
                rate_m_per_s2=(dv_mag_km / dt) * 1000.0,
            )
        )
    return out


@dataclass
class BurnEvent:
    """A cluster of contiguous high-rate sample pairs (one burn)."""

    start_sample: Sample
    end_sample: Sample
    peak_rate_m_per_s2: float
    dv_vec: Tuple[float, float, float]  # km/s (end - start velocity)
    dv_mag_ms: float  # magnitude in m/s

    @property
    def duration_seconds(self) -> float:
        return (self.end_sample.jd - self.start_sample.jd) * 86_400.0

    @property
    def midpoint_jd(self) -> float:
        return (self.start_sample.jd + self.end_sample.jd) / 2.0


def cluster_burns(
    rates: Sequence[SamplePairRate],
    threshold: float,
    max_gap_samples: int = 1,
) -> List[BurnEvent]:
    """Cluster contiguous high-rate pairs into burn events.

    `threshold` is the minimum `rate_m_per_s2` to consider "burn" (vs coast).
    `max_gap_samples` allows small sub-threshold dips inside a burn to be
    absorbed into the same cluster.
    """
    events: List[BurnEvent] = []
    i = 0
    while i < len(rates):
        if rates[i].rate_m_per_s2 <= threshold:
            i += 1
            continue

        # Found the start of a cluster.
        j = i
        last_above = i
        while j < len(rates):
            if rates[j].rate_m_per_s2 > threshold:
                last_above = j
                j += 1
                continue
            # Allow a small gap of sub-threshold samples.
            if j - last_above <= max_gap_samples:
                j += 1
                continue
            break

        start_sample = rates[i].a
        end_sample = rates[last_above].b
        peak = max(r.rate_m_per_s2 for r in rates[i : last_above + 1])
        dv_vec = (
            end_sample.vx - start_sample.vx,
            end_sample.vy - start_sample.vy,
            end_sample.vz - start_sample.vz,
        )
        dv_mag_ms = (
            (dv_vec[0] ** 2 + dv_vec[1] ** 2 + dv_vec[2] ** 2) ** 0.5 * 1000.0
        )
        events.append(
            BurnEvent(
                start_sample=start_sample,
                end_sample=end_sample,
                peak_rate_m_per_s2=peak,
                dv_vec=dv_vec,
                dv_mag_ms=dv_mag_ms,
            )
        )
        i = last_above + 1
    return events


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def jd_to_iso(jd: float) -> str:
    """Convert JD (UTC-approximated) to ISO string.

    Horizons JDTDB differs from UTC by ~69 seconds, which does not matter
    for human-readable output here. For Rust simulation, use the JD value
    directly.
    """
    # Julian Day at 2000-01-01 12:00:00 UTC = 2451545.0.
    # 1 day = 86400 seconds. Using calendar epoch Nov 17, 1858 (MJD epoch):
    #   MJD = JD - 2400000.5
    #   unix time = (MJD - 40587) * 86400
    mjd = jd - 2400000.5
    unix = (mjd - 40587.0) * 86400.0
    # Use time.gmtime to render.
    tm = time.gmtime(unix)
    frac = unix - int(unix)
    return time.strftime("%Y-%m-%dT%H:%M:%S", tm) + f".{int(frac * 1000):03d}Z"


def print_table(events: Sequence[BurnEvent]) -> None:
    print()
    print(
        f"{'#':>3}  {'Start (UTC-ish)':<24}  {'End (UTC-ish)':<24}  "
        f"{'Dur(s)':>7}  {'|Δv|(m/s)':>11}  {'Peak(m/s²)':>11}"
    )
    print("-" * 95)
    for i, ev in enumerate(events, start=1):
        print(
            f"{i:>3}  "
            f"{jd_to_iso(ev.start_sample.jd):<24}  "
            f"{jd_to_iso(ev.end_sample.jd):<24}  "
            f"{ev.duration_seconds:>7.0f}  "
            f"{ev.dv_mag_ms:>11.3f}  "
            f"{ev.peak_rate_m_per_s2:>11.5f}"
        )
    print()


def print_rust(events: Sequence[BurnEvent]) -> None:
    """Emit a Rust snippet suitable for a MANEUVERS table."""
    print()
    print("// ----- Generated by extract_burns.py — verify before using -----")
    print("// Units: ΔV in m/s (ECI J2000 equatorial), midpoint as ISO 8601.")
    print("const MANEUVERS: &[Maneuver] = &[")
    for i, ev in enumerate(events, start=1):
        mid_iso = jd_to_iso(ev.midpoint_jd)
        dvx_ms = ev.dv_vec[0] * 1000.0
        dvy_ms = ev.dv_vec[1] * 1000.0
        dvz_ms = ev.dv_vec[2] * 1000.0
        print(
            f"    // burn #{i}  peak rate = {ev.peak_rate_m_per_s2:.5f} m/s²  "
            f"duration = {ev.duration_seconds:.0f} s"
        )
        print("    Maneuver {")
        print(f'        label: "burn{i}",')
        # NOTE: the surrounding Rust struct in main.rs calls these
        # `raw_dv_eci_ms` / `raw_magnitude_ms` because the values we
        # emit here are the raw endpoint-difference Δv (propulsive +
        # gravitational drift), not the corrected propulsive-only Δv.
        # main.rs' `verify_burn` reconstructs the true propulsive Δv
        # at runtime via Method B, using these raw values only for
        # diagnostic comparison.
        print(f'        mid_epoch_iso: "{mid_iso}",')
        # Always use fixed 6-decimal format so the array reads uniformly;
        # Python's ".6" general spec mixes fixed and exponential notation.
        print(
            f"        raw_dv_eci_ms: [{dvx_ms:.6f}, {dvy_ms:.6f}, {dvz_ms:.6f}],"
        )
        print(f"        raw_magnitude_ms: {ev.dv_mag_ms:.6f},")
        print("    },")
    print("];")
    print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Scan JPL Horizons for spacecraft burn events via velocity discontinuity.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    ap.add_argument("--target", default=DEFAULT_TARGET, help="Horizons target ID")
    ap.add_argument("--center", default=DEFAULT_CENTER, help="Horizons center ID")
    # Default start is 09:05 UTC: JPL Horizons does not have Orion (target
    # -1023) ephemeris data before 2022-11-16 09:03:00 TDB (that's when
    # Orion separated from the SLS upper stage and started being tracked
    # as its own body). Asking for anything earlier gets a text error.
    ap.add_argument(
        "--start", default="2022-11-16 09:05", help="mission window start (UTC)"
    )
    ap.add_argument(
        "--stop", default="2022-12-11 17:00", help="mission window stop (UTC)"
    )
    ap.add_argument(
        "--step",
        default="5m",
        help=(
            "Horizons sample step for coarse scan. Accepts Horizons time "
            "syntax (`5m`, `1h`, `1d`) or a bare integer (= number of "
            "intervals over start→stop). Horizons does NOT accept `Ns` "
            "for seconds — use `--zoom` mode for sub-minute resolution."
        ),
    )
    ap.add_argument(
        "--threshold",
        type=float,
        default=0.1,
        help=(
            "minimum acceleration rate in m/s² to flag as burn. Default 0.1 "
            "cleanly separates propulsive events from LEO/lunar gravity "
            "(coast is O(1e-4) near Moon, O(1e-5) in free space; the OMS-E "
            "engine produces O(1e-1) or more). Lower to ~0.003 to catch "
            "sub-m/s OTC/RTC corrections, at the cost of picking up "
            "LEO/flyby gravitational drift too."
        ),
    )
    ap.add_argument(
        "--zoom",
        metavar="ISO_EPOCH",
        help=(
            "zoom mode: scan a tight window around the given epoch at fine "
            "resolution (overrides --start/--stop/--step)"
        ),
    )
    ap.add_argument(
        "--window-min",
        type=float,
        default=30.0,
        help="window width in minutes for --zoom mode (centered on ISO_EPOCH)",
    )
    ap.add_argument(
        "--zoom-step-seconds",
        type=float,
        default=30.0,
        help=(
            "zoom step in seconds; converted to an interval count so "
            "Horizons accepts it (Horizons does not recognise a raw 'Ns' "
            "step string — we emit the number of intervals instead)"
        ),
    )
    ap.add_argument(
        "--max-gap",
        type=int,
        default=2,
        help=(
            "max contiguous sub-threshold samples allowed inside a burn "
            "cluster (absorbs brief dips in apparent rate)"
        ),
    )
    ap.add_argument(
        "--rust", action="store_true", help="also emit Rust snippet for MANEUVERS"
    )
    args = ap.parse_args()

    if args.zoom:
        # Rewrite window around the zoom epoch.
        import datetime as dt

        # Accept "2022-11-25T21:50:00Z" or "2022-11-25 21:50"
        z = args.zoom.rstrip("Z").replace("T", " ")
        # Try full precision first, fall back to minute precision.
        for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"):
            try:
                base = dt.datetime.strptime(z, fmt)
                break
            except ValueError:
                continue
        else:
            raise SystemExit(f"unrecognised --zoom epoch format: {args.zoom!r}")
        half = dt.timedelta(minutes=args.window_min / 2.0)
        start = (base - half).strftime("%Y-%m-%d %H:%M")
        stop = (base + half).strftime("%Y-%m-%d %H:%M")
        # Convert desired seconds-per-step into an interval count.
        window_seconds = args.window_min * 60.0
        n_intervals = max(1, int(round(window_seconds / args.zoom_step_seconds)))
        step = str(n_intervals)
        mode = (
            f"zoom around {args.zoom} (±{args.window_min / 2:.0f} min, "
            f"{n_intervals} intervals ≈ {args.zoom_step_seconds:.0f}s step)"
        )
    else:
        start = args.start
        stop = args.stop
        step = args.step
        mode = f"coarse scan {start} → {stop} step={step}"

    print(f"Mode: {mode}", file=sys.stderr)
    csv_text = fetch_horizons(args.target, args.center, start, stop, step)
    samples = parse_vectors(csv_text)
    print(f"Parsed {len(samples)} samples", file=sys.stderr)

    rates = compute_rates(samples)
    events = cluster_burns(rates, threshold=args.threshold, max_gap_samples=args.max_gap)
    print(
        f"Detected {len(events)} burn events above {args.threshold} m/s²",
        file=sys.stderr,
    )

    print_table(events)
    if args.rust:
        print_rust(events)

    return 0


if __name__ == "__main__":
    sys.exit(main())
