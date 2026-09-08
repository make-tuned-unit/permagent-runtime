#!/usr/bin/env bash
# The oracle for harness-loc-refund-cents-drift. permagent-eval copies this
# directory OVER the finished workspace before grading, so the agent never
# saw these files and could not have weakened them. Runs with the workspace
# as cwd; exit 0 means solved.
set -uo pipefail

# The fixture crate must never build into a shared CARGO_TARGET_DIR: the
# bench may run inside a repo whose own target dir is shared with other
# sessions, and an inherited value would put bench artifacts there.
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

cp ".bench_oracle/hidden_refund_rounding.rs" "tests/hidden_refund_rounding.rs"
run_with_timeout 120 cargo test --offline --test hidden_refund_rounding
STATUS=$?
if [ $STATUS -eq 0 ]; then echo "PASS harness-loc-refund-cents-drift"; else echo "FAIL harness-loc-refund-cents-drift"; fi
exit $STATUS
