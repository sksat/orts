# /// script
# requires-python = ">=3.12"
# dependencies = ["sgp4", "requests"]
# ///
"""Generate ISS orbital decay reference fixtures from Space-Track TLE history.

Fetches historical ISS TLEs from Space-Track, identifies reboost-free decay
windows, and generates a fixture file for Rust oracle tests. The fixture
compares our drag model's predicted SMA decay against observed TLE-based decay.

Space-Track credentials via environment variables:
  SPACETRACK_USER, SPACETRACK_PASSWORD

Usage:
  uv run tools/generate_iss_decay_fixtures.py                    # analyze + generate
  uv run tools/generate_iss_decay_fixtures.py --analyze-only     # analyze only (no fixture output)
  uv run tools/generate_iss_decay_fixtures.py --select 0,3,5     # pick specific windows by index

The output fixture is committed to the repo so tests run without credentials.
Re-run this script anytime to refresh or add more windows.
"""

import argparse
import json
import math
import os
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

import requests
from sgp4.api import Satrec, WGS72

# ISS NORAD catalog ID
NORAD_ISS = 25544

# WGS72 Earth constants (matching sgp4 library)
MU_EARTH = 398600.4418  # km^3/s^2
R_EARTH = 6378.137  # km

# Reboost detection threshold: mean SMA increase > this [km]
# ISS reboosts are typically 1-10 km; 0.1 km catches small maneuvers
# while staying above TLE fitting noise (~0.05 km)
REBOOST_THRESHOLD_KM = 0.1

# Minimum window duration to be useful [days]
MIN_WINDOW_DAYS = 7

# Periods to fetch TLE data for
FETCH_PERIODS = [
    ("2019-06-01", "2020-06-30", "Solar minimum (cycle 24/25 transition)"),
    ("2024-01-01", "2025-06-30", "Solar maximum (cycle 25 peak)"),
]


def spacetrack_login(session: requests.Session, user: str, password: str) -> None:
    """Authenticate with Space-Track."""
    resp = session.post(
        "https://www.space-track.org/ajaxauth/login",
        data={"identity": user, "password": password},
    )
    resp.raise_for_status()
    if "failed" in resp.text.lower():
        print(f"Space-Track login failed: {resp.text}", file=sys.stderr)
        sys.exit(1)
    print("Space-Track login OK")


def fetch_iss_tles(
    session: requests.Session, date_start: str, date_end: str
) -> list[tuple[str, str]]:
    """Fetch ISS TLE history from Space-Track as 2-line pairs."""
    url = (
        f"https://www.space-track.org/basicspacedata/query/class/gp_history/"
        f"NORAD_CAT_ID/{NORAD_ISS}/orderby/EPOCH asc/"
        f"EPOCH/{date_start}--{date_end}/format/3le"
    )
    print(f"Fetching TLEs: {date_start} to {date_end} ...")
    resp = session.get(url)
    resp.raise_for_status()

    lines = resp.text.strip().splitlines()
    tles = []
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        if line.startswith("0 "):
            # 3LE format: line 0 (name), line 1, line 2
            if i + 2 < len(lines):
                l1 = lines[i + 1].strip()
                l2 = lines[i + 2].strip()
                if l1.startswith("1 ") and l2.startswith("2 "):
                    tles.append((l1, l2))
                    i += 3
                    continue
        elif line.startswith("1 "):
            # 2LE format fallback
            if i + 1 < len(lines):
                l2 = lines[i + 1].strip()
                if l2.startswith("2 "):
                    tles.append((line, l2))
                    i += 2
                    continue
        i += 1

    print(f"  Fetched {len(tles)} TLEs")
    return tles


def tle_to_record(line1: str, line2: str) -> dict | None:
    """Parse TLE into a record with mean SMA, epoch, and SGP4 osculating state."""
    sat = Satrec.twoline2rv(line1, line2, WGS72)
    jd_epoch = sat.jdsatepoch + sat.jdsatepochF

    # Mean motion [rev/day] → mean SMA
    # n [rad/min] = sat.no_kozai
    n_rad_s = sat.no_kozai / 60.0  # rad/s
    if n_rad_s <= 0:
        return None
    mean_sma = (MU_EARTH / (n_rad_s * n_rad_s)) ** (1 / 3)
    mean_alt = mean_sma - R_EARTH

    # SGP4 osculating state at epoch
    e, r, v = sat.sgp4(sat.jdsatepoch, sat.jdsatepochF)
    if e != 0:
        return None

    # Epoch as UTC datetime string
    year = sat.epochyr
    if year < 57:
        year += 2000
    else:
        year += 1900
    day_of_year = sat.epochdays
    epoch_dt = datetime(year, 1, 1, tzinfo=timezone.utc) + timedelta(days=day_of_year - 1)
    epoch_utc = epoch_dt.strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"

    return {
        "line1": line1,
        "line2": line2,
        "epoch_jd": jd_epoch,
        "epoch_utc": epoch_utc,
        "mean_sma_km": mean_sma,
        "mean_altitude_km": mean_alt,
        "bstar": sat.bstar,
        "position_km": list(r),
        "velocity_km_s": list(v),
    }


def find_reboosts(records: list[dict]) -> list[dict]:
    """Find all reboost events (SMA increases above threshold)."""
    reboosts = []
    for i in range(1, len(records)):
        delta_sma = records[i]["mean_sma_km"] - records[i - 1]["mean_sma_km"]
        if delta_sma > REBOOST_THRESHOLD_KM:
            reboosts.append({
                "index": i,
                "date": records[i]["epoch_utc"][:10],
                "epoch_utc": records[i]["epoch_utc"],
                "delta_sma_km": delta_sma,
                "sma_before": records[i - 1]["mean_sma_km"],
                "sma_after": records[i]["mean_sma_km"],
                "alt_before": records[i - 1]["mean_altitude_km"],
                "alt_after": records[i]["mean_altitude_km"],
            })
    return reboosts


def find_decay_windows(
    records: list[dict], min_days: float = MIN_WINDOW_DAYS
) -> list[dict]:
    """Identify reboost-free windows where SMA decreases overall."""
    if len(records) < 3:
        return []

    windows = []
    window_start = 0

    for i in range(1, len(records)):
        delta_sma = records[i]["mean_sma_km"] - records[i - 1]["mean_sma_km"]

        if delta_sma > REBOOST_THRESHOLD_KM:
            # Reboost detected — close current window if long enough
            duration_days = records[i - 1]["epoch_jd"] - records[window_start]["epoch_jd"]
            if duration_days >= min_days and (i - 1) > window_start:
                total_decay = records[window_start]["mean_sma_km"] - records[i - 1]["mean_sma_km"]
                if total_decay > 0:  # SMA actually decreased
                    # Check max internal SMA increase (TLE noise indicator)
                    sma_seq = [records[j]["mean_sma_km"] for j in range(window_start, i)]
                    max_increase = max(
                        (sma_seq[k + 1] - sma_seq[k] for k in range(len(sma_seq) - 1)),
                        default=0,
                    )
                    windows.append({
                        "start_idx": window_start,
                        "end_idx": i - 1,
                        "duration_days": duration_days,
                        "total_decay_km": total_decay,
                        "decay_rate_km_per_day": total_decay / duration_days,
                        "max_internal_increase_km": max_increase,
                        "n_tles": i - window_start,
                    })
            window_start = i

    # Check final window
    duration_days = records[-1]["epoch_jd"] - records[window_start]["epoch_jd"]
    if duration_days >= min_days:
        total_decay = records[window_start]["mean_sma_km"] - records[-1]["mean_sma_km"]
        if total_decay > 0:
            sma_seq = [records[j]["mean_sma_km"] for j in range(window_start, len(records))]
            max_increase = max(
                (sma_seq[k + 1] - sma_seq[k] for k in range(len(sma_seq) - 1)),
                default=0,
            )
            windows.append({
                "start_idx": window_start,
                "end_idx": len(records) - 1,
                "duration_days": duration_days,
                "total_decay_km": total_decay,
                "decay_rate_km_per_day": total_decay / duration_days,
                "max_internal_increase_km": max_increase,
                "n_tles": len(records) - window_start,
            })

    return windows


def build_fixture_window(
    name: str, description: str, records: list[dict], start_idx: int, end_idx: int
) -> dict:
    """Build a fixture window from a slice of TLE records."""
    initial = records[start_idx]

    tle_sequence = []
    for rec in records[start_idx : end_idx + 1]:
        tle_sequence.append({
            "line1": rec["line1"],
            "line2": rec["line2"],
            "epoch_jd": rec["epoch_jd"],
            "epoch_utc": rec["epoch_utc"],
            "mean_sma_km": rec["mean_sma_km"],
            "mean_altitude_km": rec["mean_altitude_km"],
        })

    total_decay = initial["mean_sma_km"] - records[end_idx]["mean_sma_km"]
    duration_days = records[end_idx]["epoch_jd"] - initial["epoch_jd"]

    return {
        "name": name,
        "description": description,
        "initial_tle": {
            "line1": initial["line1"],
            "line2": initial["line2"],
            "epoch_jd": initial["epoch_jd"],
            "epoch_utc": initial["epoch_utc"],
        },
        "initial_osculating": {
            "position_km": initial["position_km"],
            "velocity_km_s": initial["velocity_km_s"],
        },
        "tle_sequence": tle_sequence,
        "window_duration_days": round(duration_days, 2),
        "total_mean_sma_decay_km": round(total_decay, 4),
        "mean_decay_rate_km_per_day": round(total_decay / duration_days, 6) if duration_days > 0 else 0,
    }


def analyze_period(
    session: requests.Session, date_start: str, date_end: str, label: str
) -> tuple[list[dict], list[dict], list[dict]]:
    """Fetch TLEs, find reboosts and windows for a period. Returns (records, reboosts, windows)."""
    tles = fetch_iss_tles(session, date_start, date_end)
    if not tles:
        print(f"  No TLEs fetched for {label}!", file=sys.stderr)
        return [], [], []

    records = []
    for line1, line2 in tles:
        rec = tle_to_record(line1, line2)
        if rec is not None:
            records.append(rec)
    print(f"  Parsed {len(records)} valid records")

    if len(records) < 10:
        print(f"  Too few records for {label}", file=sys.stderr)
        return records, [], []

    sma_values = [r["mean_sma_km"] for r in records]
    print(f"  SMA range: {min(sma_values):.2f} - {max(sma_values):.2f} km")
    print(f"  Alt range: {min(sma_values) - R_EARTH:.2f} - {max(sma_values) - R_EARTH:.2f} km")

    reboosts = find_reboosts(records)
    windows = find_decay_windows(records)

    return records, reboosts, windows


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate ISS decay fixtures")
    parser.add_argument(
        "--analyze-only",
        action="store_true",
        help="Only analyze data, don't generate fixture file",
    )
    parser.add_argument(
        "--select",
        type=str,
        default=None,
        help="Comma-separated window indices to select (e.g., '0,3,5')",
    )
    args = parser.parse_args()

    user = os.environ.get("SPACETRACK_USER")
    password = os.environ.get("SPACETRACK_PASSWORD")
    if not user or not password:
        print(
            "Error: Set SPACETRACK_USER and SPACETRACK_PASSWORD environment variables.",
            file=sys.stderr,
        )
        print("  Sign up at https://www.space-track.org/auth/createAccount", file=sys.stderr)
        sys.exit(1)

    session = requests.Session()
    spacetrack_login(session, user, password)

    # Collect data from all periods
    all_records = []  # list of (period_label, records, reboosts, windows)

    for date_start, date_end, label in FETCH_PERIODS:
        print(f"\n{'='*60}")
        print(f"Period: {label} ({date_start} to {date_end})")
        print(f"{'='*60}")
        records, reboosts, windows = analyze_period(session, date_start, date_end, label)
        all_records.append((label, records, reboosts, windows))
        # Rate limiting: Space-Track asks for <30 requests/minute
        time.sleep(2)

    # Print consolidated reboost list
    print(f"\n{'='*60}")
    print("ALL DETECTED REBOOSTS (Δa > {:.1f} km)".format(REBOOST_THRESHOLD_KM))
    print(f"{'='*60}")
    for label, records, reboosts, _ in all_records:
        if not reboosts:
            continue
        print(f"\n  {label}:")
        for rb in reboosts:
            print(
                f"    {rb['date']}  Δa={rb['delta_sma_km']:+.3f} km  "
                f"alt: {rb['alt_before']:.1f} → {rb['alt_after']:.1f} km"
            )

    # Print consolidated window list with global indices
    print(f"\n{'='*60}")
    print(f"ALL DECAY WINDOWS (>= {MIN_WINDOW_DAYS} days, threshold {REBOOST_THRESHOLD_KM} km)")
    print(f"{'='*60}")
    global_windows = []
    for label, records, reboosts, windows in all_records:
        if not windows:
            continue
        print(f"\n  {label}:")
        for w in windows:
            gidx = len(global_windows)
            start_date = records[w["start_idx"]]["epoch_utc"][:10]
            end_date = records[w["end_idx"]]["epoch_utc"][:10]
            noise_flag = " ⚠" if w["max_internal_increase_km"] > 0.05 else ""
            print(
                f"    [{gidx:2d}] {start_date} to {end_date}: "
                f"{w['duration_days']:5.1f}d, "
                f"Δa={w['total_decay_km']:.3f} km "
                f"({w['decay_rate_km_per_day']:.4f} km/day), "
                f"{w['n_tles']:3d} TLEs, "
                f"max↑={w['max_internal_increase_km']:.3f} km{noise_flag}"
            )
            global_windows.append((label, records, w))

    if args.analyze_only:
        print("\n[--analyze-only] No fixture generated.")
        return

    # Select windows
    if args.select:
        selected_indices = [int(x.strip()) for x in args.select.split(",")]
    else:
        # Default: pick top 3 longest from solar minimum period
        solar_min_windows = [
            (i, gw) for i, gw in enumerate(global_windows)
            if "minimum" in gw[0].lower() or "2019" in gw[0]
        ]
        solar_min_windows.sort(key=lambda x: x[1][2]["duration_days"], reverse=True)
        selected_indices = [i for i, _ in solar_min_windows[:3]]

    if not selected_indices:
        print("No windows selected!", file=sys.stderr)
        sys.exit(1)

    print(f"\n{'='*60}")
    print(f"GENERATING FIXTURE (windows: {selected_indices})")
    print(f"{'='*60}")

    fixture_windows = []
    for i, gidx in enumerate(selected_indices):
        label, records, w = global_windows[gidx]
        start_date = records[w["start_idx"]]["epoch_utc"][:10]
        end_date = records[w["end_idx"]]["epoch_utc"][:10]
        year = start_date[:4]

        # Name based on period
        if "minimum" in label.lower() or "2019" in label:
            name = f"solar_min_{year}{'abcdefgh'[i]}"
        else:
            name = f"solar_max_{year}{'abcdefgh'[i]}"

        desc = (
            f"ISS free decay, {start_date} to {end_date}, "
            f"{w['duration_days']:.0f} days"
        )
        print(f"  {name}: {desc}")

        fw = build_fixture_window(name, desc, records, w["start_idx"], w["end_idx"])
        fixture_windows.append(fw)

    output = {
        "generator": "tools/generate_iss_decay_fixtures.py",
        "description": (
            "ISS orbital decay reference data from Space-Track TLE history. "
            "Windows are reboost-free periods. "
            "Used to validate drag model predictions (SMA decay rate)."
        ),
        "mu_earth_km3_s2": MU_EARTH,
        "r_earth_km": R_EARTH,
        "sgp4_model": "WGS72",
        "frame": "TEME",
        "note": (
            "Initial osculating state from SGP4 at window start epoch. "
            "TLE mean SMA computed from mean motion: a = (mu/n^2)^(1/3). "
            "Position in km, velocity in km/s."
        ),
        "windows": fixture_windows,
    }

    out_path = (
        Path(__file__).parent.parent
        / "orbits"
        / "tests"
        / "fixtures"
        / "iss_decay_reference.json"
    )
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output, indent=2) + "\n")
    print(f"\nWritten to {out_path}")
    print(f"  {len(fixture_windows)} windows")
    for fw in fixture_windows:
        print(
            f"    {fw['name']}: {fw['window_duration_days']} days, "
            f"decay {fw['total_mean_sma_decay_km']:.3f} km "
            f"({fw['mean_decay_rate_km_per_day']:.4f} km/day), "
            f"{len(fw['tle_sequence'])} TLEs"
        )


if __name__ == "__main__":
    main()
