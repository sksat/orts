#!/usr/bin/env python3
"""Generate a multi-sat orts.toml for the constellation-phasing bench.

Usage: python3 gen_bench_config.py N > bench_N.toml
- Emits N identical satellites on a 350 km parking orbit.
- raise_delay_s is spread over [0, sim_duration) so each satellite starts
  raising at a different point within the bench window; this exercises
  every Phase branch (Parked, FirstBurn, Coast, SecondBurn, Trim) at
  least somewhere across the ensemble.
- All satellites share the same .wasm path so the host cache compiles
  the component once and reuses it.
"""

import os
import sys
import textwrap
from pathlib import Path

SIM_DURATION_S = 3000.0
DT = 0.1
OUTPUT_INTERVAL = 100.0  # sparse to limit CSV-size scaling with N

# The generated config may live anywhere (e.g. /tmp/orts-bench), so resolve
# the .wasm to an absolute path based on this script's location.
_SCRIPT_DIR = Path(__file__).resolve().parent
WASM_PATH = str(
    _SCRIPT_DIR.parent
    / "target/wasm32-wasip2/release/orts_example_plugin_constellation_phasing.wasm"
)


def header() -> str:
    return textwrap.dedent(f"""\
        body = "earth"
        dt = {DT}
        output_interval = {OUTPUT_INTERVAL}
        duration = {SIM_DURATION_S}
        epoch = "2024-01-01T00:00:00Z"
        atmosphere = "none"
        """)


def satellite_block(i: int, delay_s: float) -> str:
    return textwrap.dedent(f"""\

        [[satellites]]
        id = "sat-{i}"
        sensors = ["gyroscope", "star_tracker"]

        [satellites.orbit]
        type = "circular"
        altitude = 350

        [satellites.attitude]
        inertia_diag = [100, 100, 100]
        mass = 500
        initial_quaternion = [1.0, 0.0, 0.0, 0.0]
        initial_angular_velocity = [0.0, 0.0, 0.0]

        [satellites.controller]
        type = "wasm"
        path = "{WASM_PATH}"

        [satellites.controller.config]
        target_altitude_km = 550.0
        raise_delay_s = {delay_s}
        mu_km3_s2 = 398600.4418
        deadband_km = 5.0
        num_thrusters = 1
        num_rws = 3
        kp = 500.0
        kd = 150.0
        sample_period = 0.1

        [satellites.reaction_wheels]
        type = "three_axis"
        inertia = 0.05
        max_momentum = 100.0
        max_torque = 10.0

        [satellites.thruster]
        dry_mass = 100.0

        [[satellites.thruster.thrusters]]
        thrust_n = 5000.0
        isp_s = 230.0
        direction_body = [0.0, 1.0, 0.0]
        """)


def main() -> None:
    if len(sys.argv) != 2:
        sys.stderr.write("usage: gen_bench_config.py N\n")
        sys.exit(2)
    n = int(sys.argv[1])
    if n < 1:
        sys.stderr.write("N must be >= 1\n")
        sys.exit(2)

    out = [header()]
    for i in range(n):
        # Spread delays over [0, 0.9 * duration) so the last satellite still
        # starts its burn within the bench window.
        delay = (0.9 * SIM_DURATION_S) * (i / max(n - 1, 1)) if n > 1 else 0.0
        out.append(satellite_block(i, round(delay, 1)))
    sys.stdout.write("".join(out))


if __name__ == "__main__":
    main()
