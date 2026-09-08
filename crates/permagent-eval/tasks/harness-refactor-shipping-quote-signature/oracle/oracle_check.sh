#!/usr/bin/env bash
# The oracle for harness-refactor-shipping-quote-signature. permagent-eval
# copies this directory OVER the finished workspace before grading, so the
# agent never saw these files and could not have weakened them. Runs with
# the workspace as cwd; exit 0 means solved.
set -uo pipefail

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

mkdir -p oracle_run
cp ".bench_oracle/hidden_check.ts" "oracle_run/hidden_check.ts"
run_with_timeout 60 node --experimental-strip-types "oracle_run/hidden_check.ts"
STATUS=$?
if [ $STATUS -eq 0 ]; then echo "PASS harness-refactor-shipping-quote-signature"; else echo "FAIL harness-refactor-shipping-quote-signature"; fi
exit $STATUS
