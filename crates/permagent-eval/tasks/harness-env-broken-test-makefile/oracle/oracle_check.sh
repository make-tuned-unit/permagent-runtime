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

# Lane K11: this used to be `grep -q "test/run\.ts" Makefile` before `make
# test` ever ran, rejecting any fix that routed through `npm test` ->
# `package.json`'s "test" script -> test/run.ts instead of naming the path
# literally in the Makefile. That's a legitimate, arguably more idiomatic
# fix and it was failing the oracle without ever being executed. Grade
# behavior instead: run the real command and require the real test file's
# marker in its output, so *how* the fix routes to test/run.ts no longer
# matters -- only that it actually does.
#
# The one thing worth keeping a static guard for is the decoy-file attack
# this fixture was built to block: the seed Makefile references the missing
# "tests/run.ts" (note the plural -- there is no such directory; the real
# suite lives at "test/run.ts", restored pristine above). A worker could
# leave that broken reference in place and instead drop a fake file at the
# still-broken old path that just prints the marker, making `make test`
# "pass" without ever running the real suite. Guard against the tampering
# artifact (a file that should not exist), not against how the Makefile is
# phrased -- that still lets direct and indirect (npm/package.json) fixes
# through.
if [ -e "tests/run.ts" ]; then
  echo "FAIL harness-env-broken-test-makefile (decoy file present at broken path tests/run.ts)"
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
