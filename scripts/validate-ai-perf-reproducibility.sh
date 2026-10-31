#!/usr/bin/env bash
# Strict pre-baseline reproducibility + CI-budget validation (M15).
#
# Generates a fresh median-of-K perf baseline, runs N further median-of-K gate
# runs against it, then applies the MARGIN gate: every counter's worst observed
# value must stay within PERF_REPRO_MARGIN_FRACTION of its FAIL headroom. It also
# TIMES the cold build and every gate run so the executor can apply the CI-budget
# check with MEASURED numbers rather than asserted estimates.
#
# Runs the SERVER-RELEASE binary — the authoritative gate profile CI runs
# (`cargo ai-perf-gate`). This script isolates CARGO_TARGET_DIR to target/ai, so
# the parent's current_exe() resolves to target/ai/server-release/ai-perf-gate
# and the K spawned children are server-release too (profile-consistent parent
# and children).
#
# The profile here is load-bearing and must track the `cargo ai-perf-gate` alias:
# this script hardcodes a target/<profile>/ path while the binary re-spawns ITSELF
# by current_exe(). If the alias and this path ever name different profiles, the
# script silently measures the wrong binary (or a stale one) instead of failing.
#
# ONLY commit the generated baseline if this script PASSES (margin + all N band
# runs exit 0) AND the measured CI-budget check passes. Otherwise escalate per
# the plan — never widen the band here.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="$ROOT/target/ai"     # isolated: no Tilt lock contention (mirrors ai-perf-gate.sh)

# A previous target/ai build would invalidate the cold-build claim. Preserve it
# and fail instead of deleting an existing cache or measuring a warm rebuild.
if [ -e "$CARGO_TARGET_DIR" ] || [ -L "$CARGO_TARGET_DIR" ]; then
  if [ ! -d "$CARGO_TARGET_DIR" ] || [ -L "$CARGO_TARGET_DIR" ]; then
    echo "Cold target must be a real directory: $CARGO_TARGET_DIR" >&2
    exit 1
  fi
  existing=$(find "$CARGO_TARGET_DIR" -mindepth 1 -print -quit)
  if [ -n "$existing" ]; then
    echo "Cold build requires an empty $CARGO_TARGET_DIR; preserve/retire prior output first." >&2
    exit 1
  fi
fi

# SERVER-RELEASE profile — the authoritative gate profile CI runs (`cargo ai-perf-gate`).
# Time the cold isolated build: cold-isolated >= CI's warm rust-ai-gate cache
# hit, so this is a conservative T_build ceiling for the budget check.
build_start=$(date +%s)
cargo build --locked --profile server-release --bin ai-perf-gate
build_seconds=$(( $(date +%s) - build_start ))
echo "T_build (cold isolated server-release build) = ${build_seconds}s"
BIN="$CARGO_TARGET_DIR/server-release/ai-perf-gate"   # current_exe() -> server-release children

"$BIN" --refresh-baseline                     # 1) generate the median-of-K baseline

N=25                                          # keep in sync with PERF_REPRO_VALIDATION_RUNS
inputs=(); band_fail=0; max_run_seconds=0
for i in $(seq 1 "$N"); do                    # 2) N further median-of-K gate runs vs the baseline
  out="$ROOT/target/ai-perf-repro-$i.json"
  start=$(date +%s)
  if ! "$BIN" --current-output "$out"; then band_fail=1; fi   # existing band gate (weak Bernoulli check)
  elapsed=$(( $(date +%s) - start ))
  echo "run $i wall=${elapsed}s"                            # T_run sample (= PERF_SAMPLE_COUNT children)
  if [ "$elapsed" -gt "$max_run_seconds" ]; then max_run_seconds=$elapsed; fi
  inputs+=(--repro-input "$out")
done

# 3) MARGIN GATE — exit 0 iff all counters within 50% headroom. Capture the code
# without letting `set -e` abort before the summary (the margin table itself is
# printed by the binary regardless).
if "$BIN" --repro-report "${inputs[@]}"; then margin_rc=0; else margin_rc=$?; fi

if [ "$band_fail" -ne 0 ] || [ "$margin_rc" -ne 0 ]; then
  echo "REPRO VALIDATION FAILED (band_fail=$band_fail margin_rc=$margin_rc) — DO NOT COMMIT baseline; escalate."
  exit 1
fi
echo "REPRO VALIDATION PASSED (margin+band) — checking the measured cold-build budget:"
python3 "$ROOT/scripts/ai-perf-budget.py" --build-seconds "$build_seconds" --max-run-seconds "$max_run_seconds"
echo "  Budget passed: 25 minutes by default; 60 only for the private saget CI runner."
echo "  (The former 'fall back to --release' option is gone: all four gate"
echo "   authorities now build server-release, and --release is the WASM-size profile.)"
