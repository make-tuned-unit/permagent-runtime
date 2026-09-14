#!/usr/bin/env bash
# The oracle for harness-loc-refund-cents-drift. permagent-eval copies this
# directory OVER the finished workspace before grading, so the agent never
# saw these files and could not have weakened them. Runs with the workspace
# as cwd; exit 0 means solved.
set -uo pipefail

# Lane K7: permagent-eval's runner (Lane R18c) SETS CARGO_TARGET_DIR to this
# run's own scoped directory before spawning this script, specifically so
# `cargo test` below builds in isolation. Do NOT unset it: cargo's fallback
# default (a bare `target/` next to Cargo.toml) can itself resolve into a
# dev checkout's SHARED target dir when a worktree's `target/` is a symlink
# for build-cache sharing -- exactly the collision this variable exists to
# avoid. Inherit it as set.

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

cp ".bench_oracle/hidden_refund_rounding.rs" "tests/hidden_refund_rounding.rs"
# Lane K7: 260s (not 120s) -- a cold build into a fresh per-run
# CARGO_TARGET_DIR (no warm target-dir cache, only the shared CARGO_HOME
# registry cache) takes longer than a warm rebuild, and the outer
# task.yaml `oracle_timeout_secs: 300` gives this single step room
# underneath it.
run_with_timeout 260 cargo test --offline --test hidden_refund_rounding
STATUS=$?
if [ $STATUS -eq 0 ]; then echo "PASS harness-loc-refund-cents-drift"; else echo "FAIL harness-loc-refund-cents-drift"; fi
exit $STATUS
