#!/usr/bin/env bash
# The oracle for task-06. permagent-eval copies this directory OVER the finished
# workspace before grading, so the agent never saw these files and could not
# have weakened them. Runs with the workspace as cwd; exit 0 means solved.
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
    # This box has neither; perl's alarm is the portable stand-in.
    perl -e "alarm shift(@ARGV); exec { \$ARGV[0] } @ARGV" "$secs" "$@"
  fi
}

cp ".bench_oracle/hidden_task_06.rs" "rs/tests/hidden_task_06.rs"
run_with_timeout 260 cargo test --offline --manifest-path "rs/Cargo.toml" --test hidden_task_06
STATUS=$?
if [ $STATUS -eq 0 ]; then echo "PASS task-06"; else echo "FAIL task-06"; fi
exit $STATUS
