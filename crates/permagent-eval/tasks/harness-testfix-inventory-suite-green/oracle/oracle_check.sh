#!/usr/bin/env bash
# The oracle for harness-testfix-inventory-suite-green. permagent-eval copies
# this directory OVER the finished workspace before grading -- restoring
# tests/test_inventory.py to its pristine assertions (so the agent cannot
# "fix" the suite by weakening, skipping, or deleting a check) -- and runs
# with the workspace as cwd; exit 0 means solved.
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

run_with_timeout 30 python3 -m tests.test_inventory
VISIBLE=$?

# A second, independent layer of assertions that doesn't rely on the
# unittest test-file machinery at all (so it can't be defeated by a
# workspace-level monkeypatch of unittest.TestCase or similar).
cp ".bench_oracle/hidden_extra_checks.py" "hidden_extra_checks.py"
run_with_timeout 30 python3 hidden_extra_checks.py
HIDDEN=$?

if [ $VISIBLE -eq 0 ] && [ $HIDDEN -eq 0 ]; then
  echo "PASS harness-testfix-inventory-suite-green"
  exit 0
else
  echo "FAIL harness-testfix-inventory-suite-green (visible=$VISIBLE hidden=$HIDDEN)"
  exit 1
fi
