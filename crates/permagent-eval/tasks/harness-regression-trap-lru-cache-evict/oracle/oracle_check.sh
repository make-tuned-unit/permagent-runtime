#!/usr/bin/env bash
# The oracle for harness-regression-trap-lru-cache-evict. permagent-eval
# copies this directory OVER the finished workspace before grading, so the
# agent never saw these files and could not have weakened them. Runs with
# the workspace as cwd; exit 0 means solved.
set -uo pipefail

unset CARGO_TARGET_DIR

run_with_timeout() {
  local secs="$1"; shift
  if command -v timeout >/dev/null 2>&1; then
    timeout "$secs" "$@"
  elif command -v gtimeout >/dev/null 2>&1; then
    gtimeout "$secs" "$@"
  else
    perl -e "alarm shift(@ARGV); exec { \$ARGV[0] } @ARGV" "$secs" "$@"
  fi
}

# First: the seed's own visible test must go green (capacity is enforced).
run_with_timeout 60 cargo test --offline --test lru_tests
VISIBLE=$?

# Second: the hidden regression-trap invariant -- eviction must be LRU-correct,
# not just "shrink back to capacity by any means" (e.g. clearing everything or
# evicting the wrong entry would satisfy `lru_tests` but fail this).
cp ".bench_oracle/hidden_lru_eviction_order.rs" "tests/hidden_lru_eviction_order.rs"
run_with_timeout 60 cargo test --offline --test hidden_lru_eviction_order
HIDDEN=$?

if [ $VISIBLE -eq 0 ] && [ $HIDDEN -eq 0 ]; then
  echo "PASS harness-regression-trap-lru-cache-evict"
  exit 0
else
  echo "FAIL harness-regression-trap-lru-cache-evict (visible=$VISIBLE hidden=$HIDDEN)"
  exit 1
fi
