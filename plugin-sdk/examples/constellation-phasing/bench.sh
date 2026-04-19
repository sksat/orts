#!/usr/bin/env bash
# Benchmark multi-satellite WASM plugin scaling.
#
# Usage:
#   ./bench.sh [sync|async|auto]  [quick|full]
#
# - full mode: N = 1, 2, 4, 8, 16, 32, 64 with --warmup 2 --runs 7 (README values)
# - quick mode: N = 1, 8, 64 with --warmup 1 --runs 3 (dev iteration)
#
# Prereq:
#   cargo build --target wasm32-wasip2 --release   # (in this directory)
#   hyperfine (github.com/sharkdp/hyperfine)
#   python3

set -euo pipefail

BACKEND="${1:-auto}"
MODE="${2:-full}"
WORK=/tmp/orts-bench

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
cd "$SCRIPT_DIR"

mkdir -p "$WORK"

if [[ "$MODE" == "quick" ]]; then
    NS=(1 8 64)
    HF_ARGS=(--warmup 1 --runs 3)
elif [[ "$MODE" == "full" ]]; then
    NS=(1 2 4 8 16 32 64)
    HF_ARGS=(--warmup 2 --runs 7)
else
    echo "Unknown mode: $MODE (expected quick|full)" >&2
    exit 2
fi

case "$BACKEND" in
    sync|async|auto) ;;
    *) echo "Unknown backend: $BACKEND (expected sync|async|auto)" >&2; exit 2 ;;
esac

for N in "${NS[@]}"; do
    python3 gen_bench_config.py "$N" > "$WORK/bench_N${N}.toml"
done

NS_CSV=$(IFS=, ; echo "${NS[*]}")

# Use the orts-cli binary directly (not `cargo run`) so hyperfine measures
# the simulation only, not cargo's up-to-date check. The default .rrd output
# is fine for bench — we don't read it back, we only care about wall-clock.
BIN="$(git rev-parse --show-toplevel)/target/release/orts"
if [[ ! -x "$BIN" ]]; then
    echo "Building orts-cli release binary..." >&2
    cargo build --manifest-path="$(git rev-parse --show-toplevel)/Cargo.toml" -p orts-cli --release
fi

hyperfine "${HF_ARGS[@]}" \
    -L n "$NS_CSV" \
    --export-markdown "$WORK/result_${BACKEND}_${MODE}.md" \
    --export-json "$WORK/result_${BACKEND}_${MODE}.json" \
    "$BIN run --config $WORK/bench_N{n}.toml --plugin-backend ${BACKEND} --output $WORK/out_N{n}.rrd"

echo
echo "=== Results saved ==="
echo "markdown: $WORK/result_${BACKEND}_${MODE}.md"
echo "json:     $WORK/result_${BACKEND}_${MODE}.json"
