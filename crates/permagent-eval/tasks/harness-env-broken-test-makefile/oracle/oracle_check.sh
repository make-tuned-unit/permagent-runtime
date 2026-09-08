#!/usr/bin/env bash
# The oracle for harness-env-broken-test-makefile. permagent-eval copies this
# directory OVER the finished workspace before grading -- restoring
# test/run.ts to its pristine content so the agent cannot "fix" the build by
# weakening the test instead of the Makefile -- and runs with the workspace
# as cwd; exit 0 means solved.
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

# The only real test entrypoint the repo ships is test/run.ts (restored
# pristine above); the Makefile must actually invoke that path rather than a
# decoy file dropped at the (still-broken) old path the Makefile used to
# reference. This is a literal substring check, not "tests/run.ts" (note: no
# 's'), so a genuinely-fixed Makefile passes and an unfixed one does not.
if ! grep -q "test/run\.ts" Makefile; then
  echo "FAIL harness-env-broken-test-makefile (Makefile does not reference test/run.ts)"
  exit 1
fi

OUTPUT=$(run_with_timeout 30 make test 2>&1)
STATUS=$?
echo "$OUTPUT"

# Trust `make test`'s own exit code, AND require the pristine test file's
# marker in stdout -- this defeats a Makefile "fix" that just redefines the
# test target to `@true` (or similar) without actually running the tests.
if [ "$STATUS" -eq 0 ] && echo "$OUTPUT" | grep -q "ALL_TESTS_PASSED_MARKER_9f3a"; then
  echo "PASS harness-env-broken-test-makefile"
  exit 0
else
  echo "FAIL harness-env-broken-test-makefile"
  exit 1
fi
