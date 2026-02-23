# /// script
# requires-python = ">=3.10"
# dependencies = ["requests"]
# ///
"""Generate trimmed CSSI space weather fixture for oracle tests.

Downloads SW-All.txt from CelesTrak and extracts only the date ranges
needed by the test suite. The output file is a valid CSSI-format file
parseable by both our Rust CssiData::parse() and Orekit's
CssiSpaceWeatherData.

Required date ranges (with margin for Ap 3-hour history):
  - 2019-06-28 to 2019-08-18  (ISS decay 2019a)
  - 2019-09-10 to 2019-11-10  (ISS decay 2019b)
  - 2020-04-14 to 2020-07-02  (ISS decay 2020c)
  - 2024-03-09 to 2024-06-24  (ISS decay 2024d + Orekit propagation + density)

Output: tobari/tests/fixtures/cssi_test_weather.txt
        orbits/tests/fixtures/cssi_test_weather.txt (copy)
Run:    uv run tools/generate_cssi_test_fixture.py
"""

import shutil
import sys
from datetime import date, timedelta
from pathlib import Path

import requests

SW_ALL_URL = "https://celestrak.org/SpaceData/SW-All.txt"
CACHE_DIR = Path.home() / ".cache" / "orts"
CACHE_FILE = CACHE_DIR / "SW-All.txt"

# Date ranges to include (inclusive, with margin for Ap history lookback)
DATE_RANGES = [
    (date(2019, 6, 28), date(2019, 8, 18)),   # ISS decay 2019a
    (date(2019, 9, 10), date(2019, 11, 10)),   # ISS decay 2019b
    (date(2020, 4, 14), date(2020, 7, 2)),     # ISS decay 2020c
    (date(2024, 3, 9), date(2024, 6, 24)),     # ISS decay 2024d + Orekit prop + density
]


def download_sw_all() -> str:
    """Download SW-All.txt from CelesTrak, using cache if available."""
    CACHE_DIR.mkdir(parents=True, exist_ok=True)

    if CACHE_FILE.exists():
        age = CACHE_FILE.stat().st_mtime
        import time
        if time.time() - age < 7 * 86400:  # 7-day cache
            print(f"Using cached {CACHE_FILE}")
            return CACHE_FILE.read_text()

    print(f"Downloading {SW_ALL_URL}...")
    resp = requests.get(SW_ALL_URL, timeout=120)
    resp.raise_for_status()
    CACHE_FILE.write_text(resp.text)
    print(f"Cached to {CACHE_FILE} ({len(resp.text)} bytes)")
    return resp.text


def parse_date_from_line(line: str) -> date | None:
    """Extract date from a CSSI data line (columns 1-4=year, 5-6=month, 7-8=day)."""
    stripped = line.strip()
    if len(stripped) < 8:
        return None
    try:
        year = int(stripped[0:4])
        month = int(stripped[5:7])
        day = int(stripped[8:10])
        if 1900 <= year <= 2100 and 1 <= month <= 12 and 1 <= day <= 31:
            return date(year, month, day)
    except (ValueError, IndexError):
        return None
    return None


def is_in_range(d: date) -> bool:
    """Check if a date falls within any of the required ranges."""
    return any(start <= d <= end for start, end in DATE_RANGES)


def extract_fixture(sw_text: str) -> str:
    """Extract relevant records from SW-All.txt and wrap in valid CSSI format.

    Orekit's CssiSpaceWeatherData parser requires the DATATYPE/VERSION header
    and NUM_*_POINTS metadata lines to parse correctly.
    """
    lines = sw_text.splitlines()

    # Find section markers and data lines
    observed_records = []
    predicted_records = []
    in_observed = False
    in_predicted = False

    for line in lines:
        if "BEGIN OBSERVED" in line:
            in_observed = True
            in_predicted = False
            continue
        elif "END OBSERVED" in line:
            in_observed = False
            continue
        elif "BEGIN DAILY_PREDICTED" in line:
            in_predicted = True
            in_observed = False
            continue
        elif "END DAILY_PREDICTED" in line:
            in_predicted = False
            continue

        if not (in_observed or in_predicted):
            continue

        d = parse_date_from_line(line)
        if d is not None and is_in_range(d):
            if in_observed:
                observed_records.append(line)
            elif in_predicted:
                predicted_records.append(line)

    if not observed_records and not predicted_records:
        print("ERROR: No records found in the specified date ranges!", file=sys.stderr)
        sys.exit(1)

    # Build output file with proper CSSI header (required by Orekit parser)
    output_lines = []
    output_lines.append("DATATYPE CssiSpaceWeather")
    output_lines.append("VERSION 1.2 ")
    output_lines.append(f"NUM_OBSERVED_POINTS {len(observed_records)}")
    output_lines.append("BEGIN OBSERVED")
    output_lines.extend(observed_records)
    output_lines.append("END OBSERVED")

    if predicted_records:
        output_lines.append("")
        output_lines.append(f"NUM_DAILY_PREDICTED_POINTS {len(predicted_records)}")
        output_lines.append("BEGIN DAILY_PREDICTED")
        output_lines.extend(predicted_records)
        output_lines.append("END DAILY_PREDICTED")

    return "\n".join(output_lines) + "\n"


def main():
    sw_text = download_sw_all()

    print("\nExtracting records for date ranges:")
    for start, end in DATE_RANGES:
        print(f"  {start} to {end}")

    fixture_text = extract_fixture(sw_text)
    n_lines = len([l for l in fixture_text.splitlines() if l.strip() and not l.startswith(("BEGIN", "END"))])
    print(f"\nExtracted {n_lines} data records")

    # Write to tobari fixtures
    out_tobari = Path(__file__).parent.parent / "tobari" / "tests" / "fixtures" / "cssi_test_weather.txt"
    out_tobari.parent.mkdir(parents=True, exist_ok=True)
    out_tobari.write_text(fixture_text)
    print(f"Written to {out_tobari}")

    # Copy to orbits fixtures
    out_orbits = Path(__file__).parent.parent / "orbits" / "tests" / "fixtures" / "cssi_test_weather.txt"
    out_orbits.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(out_tobari, out_orbits)
    print(f"Copied to {out_orbits}")

    # Print summary
    print(f"\nFixture size: {len(fixture_text)} bytes, {n_lines} records")


if __name__ == "__main__":
    main()
