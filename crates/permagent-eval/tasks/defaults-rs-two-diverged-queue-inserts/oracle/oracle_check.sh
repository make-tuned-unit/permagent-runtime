#!/usr/bin/env bash
# The oracle for task-06. permagent-eval copies this directory OVER the finished
# workspace before grading, so the agent never saw these files and could not
# have weakened them. Runs with the workspace as cwd; exit 0 means solved.
set -uo pipefail

# The fixture crate must never build into a shared CARGO_TARGET_DIR: the
# bench runs inside a repo whose own target dir is shared with other
# sessions, and an inherited value would put bench artifacts there.
unset CARGO_TARGET_DIR

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
run_with_timeout 180 cargo test --offline --manifest-path "rs/Cargo.toml" --test hidden_task_06
STATUS=$?
if [ $STATUS -eq 0 ]; then echo "PASS task-06"; else echo "FAIL task-06"; fi
exit $STATUS
