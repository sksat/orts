#!/usr/bin/env bash
# E2E test: verify NOS3 ADCS control modes.
#
# Usage:
#   ./test.sh              # run all mode tests
#   ./test.sh bdot         # B-dot detumbling only
#   ./test.sh sunsafe      # Sun-Safe pointing only
#   ./test.sh inertial     # Inertial 3-axis control only
#
# Prerequisites:
#   - cargo-component installed
#   - wasi-libc / wasi-compiler-rt installed
#   - orts CLI buildable from workspace root

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

CLI_MANIFEST="$SCRIPT_DIR/../../../cli/Cargo.toml"
TMPCSV=$(mktemp /tmp/nos3_adcs_test_XXXXXX.csv)
TMPTOML=$(mktemp /tmp/nos3_adcs_test_XXXXXX.toml)
RESULTS_DIR="$SCRIPT_DIR/results"
mkdir -p "$RESULTS_DIR"
trap 'rm -f "$TMPCSV" "$TMPTOML"' EXIT

# ── Build ──

build() {
    echo "=== Building nos3-adcs plugin ==="
    cargo component build

    if [ -n "${ORTS_BIN:-}" ] && [ -x "$ORTS_BIN" ]; then
        echo "=== Using pre-built orts CLI: $ORTS_BIN ==="
    else
        echo "=== Building orts CLI ==="
        cargo build --manifest-path "$CLI_MANIFEST" 2>/dev/null
        ORTS_BIN="$(cargo metadata --manifest-path "$CLI_MANIFEST" --format-version 1 2>/dev/null | \
            python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/orts"
    fi
}

# ── Run simulation and extract angular velocity norm ──

run_sim() {
    local config="$1" label="$2"
    local csv_out="$RESULTS_DIR/${label}.csv"

    # Single simulation run → CSV (timed)
    local t0 t1 elapsed_ms
    t0=$(date +%s%N)
    "$ORTS_BIN" run --config "$config" --format csv --output stdout > "$csv_out" 2>/dev/null
    t1=$(date +%s%N)
    elapsed_ms=$(( (t1 - t0) / 1000000 ))
    cp "$csv_out" "$TMPCSV"

    # Single-pass awk: extract initial and final |omega|, append elapsed_ms
    awk -F, -v elapsed="$elapsed_ms" '
        /^#/ { next }
        !got_first { first_wx=$14; first_wy=$15; first_wz=$16; got_first=1 }
        { last_wx=$14; last_wy=$15; last_wz=$16 }
        END {
            printf "%.10f %.10f %s\n", \
                sqrt(first_wx^2 + first_wy^2 + first_wz^2), \
                sqrt(last_wx^2 + last_wy^2 + last_wz^2), \
                elapsed
        }
    ' "$TMPCSV"
}

check_reduction() {
    local label="$1" threshold="$2" initial="$3" final="$4" elapsed="${5:-}"
    local time_str=""
    if [ -n "$elapsed" ]; then
        time_str=" [${elapsed}ms]"
    fi
    local pass
    pass=$(awk "BEGIN { print ($final < $initial * $threshold) ? 1 : 0 }")
    if [ "$pass" -eq 1 ]; then
        local pct
        pct=$(awk "BEGIN { printf \"%.1f\", (1 - $final / $initial) * 100 }")
        echo "  $label: |omega| $initial -> $final rad/s (${pct}% reduction)${time_str}"
        return 0
    else
        echo "  $label: FAIL |omega| $initial -> $final rad/s${time_str}"
        echo "    Expected: final < initial * $threshold"
        return 1
    fi
}

# ── Generate TOML config ──

gen_config() {
    local mode="$1" duration="$2" dt="$3" sample_period="$4"
    local extra_config="${5:-}"
    local omega="${6:-0.15, -0.10, 0.12}"
    cat > "$TMPTOML" << EOF
body = "earth"
dt = $dt
output_interval = 1.0
duration = $duration
epoch = "2026-04-18T12:00:00Z"

[[satellites]]
id = "test"
sensors = ["magnetometer", "gyroscope", "star_tracker", "sun_sensor"]

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500
initial_quaternion = [0.966, 0, 0.259, 0]
initial_angular_velocity = [$omega]

[satellites.controller]
type = "wasm"
path = "target/wasm32-wasip1/debug/orts_example_plugin_nos3_adcs.wasm"

[satellites.controller.config]
sample_period = $sample_period
initial_mode = $mode
$extra_config

[satellites.reaction_wheels]
type = "three_axis"
inertia = 0.01
max_momentum = 1.0
max_torque = 0.5

[satellites.magnetorquers]
type = "three_axis"
max_moment = 10.0
EOF
}

# ── Mode tests ──

test_bdot() {
    echo "=== B-dot detumbling (mode 1, 2 orbits ~3h) ==="
    gen_config 1 11100.0 0.1 1.0 "bdot_kb = 1e4
bdot_b_range = 1e-9"
    read -r initial final elapsed < <(run_sim "$TMPTOML" bdot)
    check_reduction "B-dot" 0.05 "$initial" "$final" "$elapsed"
}

test_sunsafe() {
    echo "=== Sun-Safe pointing (mode 2, 600s) ==="
    gen_config 2 600.0 0.1 1.0 "sunsafe_kp = [0.01, 0.01, 0.01]
sunsafe_kr = [0.1, 0.1, 0.1]
sunsafe_sside = [1.0, 0.0, 0.0]
sunsafe_vmax = 0.01
momentum_management = true"
    read -r initial final elapsed < <(run_sim "$TMPTOML" sunsafe)
    check_reduction "Sun-Safe" 0.2 "$initial" "$final" "$elapsed"
}

test_inertial() {
    echo "=== Inertial 3-axis control (mode 3, 120s, post-detumble ω) ==="
    gen_config 3 120.0 0.01 0.1 "inertial_kp = [0.5, 0.5, 0.5]
inertial_kr = [2.0, 2.0, 2.0]
inertial_phi_err_max = 1.0
momentum_management = true" "0.01, -0.005, 0.008"
    read -r initial final elapsed < <(run_sim "$TMPTOML" inertial)
    check_reduction "Inertial" 0.01 "$initial" "$final" "$elapsed"
}

# ── Main ──

build

MODES="${1:-all}"
FAIL=0

case "$MODES" in
    bdot)     test_bdot     || FAIL=1 ;;
    sunsafe)  test_sunsafe  || FAIL=1 ;;
    inertial) test_inertial || FAIL=1 ;;
    all)
        test_bdot     || FAIL=1
        test_sunsafe  || FAIL=1
        test_inertial || FAIL=1
        ;;
    *)
        echo "Unknown mode: $MODES"
        echo "Usage: $0 [bdot|sunsafe|inertial|all]"
        exit 1
        ;;
esac

if [ "$FAIL" -eq 0 ]; then
    echo "=== ALL PASS ==="

    # Generate plots from saved CSV if uv is available
    if command -v uv &>/dev/null; then
        echo "=== Generating plots ==="
        for mode in bdot sunsafe inertial; do
            csv="$RESULTS_DIR/${mode}.csv"
            png="$RESULTS_DIR/${mode}.png"
            if [ -f "$csv" ]; then
                uv run python3 plot.py "$csv" --save "$png" --title "NOS3 ADCS: $mode"
                echo "  Saved $png"
            fi
        done
    fi
else
    echo "=== SOME TESTS FAILED ==="
    exit 1
fi
