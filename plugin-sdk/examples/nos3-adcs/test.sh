#!/usr/bin/env bash
# E2E test: verify NOS3 ADCS B-dot detumbling reduces angular velocity.
#
# Usage:
#   ./test.sh
#
# Prerequisites:
#   - cargo-component installed
#   - wasi-libc / wasi-compiler-rt installed (for C → wasm32 cross-compilation)
#   - orts CLI buildable from workspace root

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

CLI_MANIFEST="$SCRIPT_DIR/../../../cli/Cargo.toml"
TMPCSV=$(mktemp /tmp/nos3_adcs_test_XXXXXX.csv)
trap 'rm -f "$TMPCSV"' EXIT

echo "=== Building nos3-adcs plugin ==="
cargo component build

echo "=== Building orts CLI ==="
cargo build --manifest-path "$CLI_MANIFEST" 2>/dev/null
ORTS_BIN="$(cargo metadata --manifest-path "$CLI_MANIFEST" --format-version 1 2>/dev/null | \
    python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/orts"

echo "=== Running B-dot detumbling simulation (600s) ==="
"$ORTS_BIN" run --config orts.toml --format csv --output stdout > "$TMPCSV"

echo "=== Checking angular velocity reduction ==="
# Use awk to extract initial and final |omega| in a single pass (no head/tail pipes)
read -r INITIAL FINAL < <(awk -F, '
    /^#/ { next }
    NR == 1 || !header_done {
        # first data row
        if ($0 !~ /^#/) {
            header_done = 1
            first_wx = $14; first_wy = $15; first_wz = $16
        }
    }
    !/^#/ { last_wx = $14; last_wy = $15; last_wz = $16 }
    END {
        initial = sqrt(first_wx^2 + first_wy^2 + first_wz^2)
        final_v = sqrt(last_wx^2 + last_wy^2 + last_wz^2)
        printf "%.10f %.10f\n", initial, final_v
    }
' "$TMPCSV")

echo "  Initial |omega| = $INITIAL rad/s"
echo "  Final   |omega| = $FINAL rad/s"

# Assert: angular velocity norm decreased by at least 20%
PASS=$(awk "BEGIN { print ($FINAL < $INITIAL * 0.8) ? 1 : 0 }")

if [ "$PASS" -eq 1 ]; then
    REDUCTION=$(awk "BEGIN { printf \"%.1f\", (1 - $FINAL / $INITIAL) * 100 }")
    echo "  B-dot detumbling working: ${REDUCTION}% reduction"
    echo "=== PASS ==="
else
    echo "  Angular velocity did not decrease sufficiently"
    echo "    Expected: final < initial * 0.8"
    echo "    Got: $FINAL >= $INITIAL * 0.8"
    echo "=== FAIL ==="
    exit 1
fi
