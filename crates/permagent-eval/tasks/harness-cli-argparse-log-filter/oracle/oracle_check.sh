#!/usr/bin/env bash
# The oracle for harness-cli-argparse-log-filter. permagent-eval copies this
# directory OVER the finished workspace before grading, so the agent never
# saw these checks and could not have weakened them. Runs with the workspace
# as cwd; exit 0 means solved.
#
# Pure bash + direct CLI invocations (no python oracle script) so the
# Python-oracle import allowlist never comes into play: every check just
# shells out to `python3 -m logfilter ...` and inspects its exit code and
# stdout/stderr.
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

LOG="sample_logs/app.log"
FAILS=0

check() {
  local name="$1"; shift
  if "$@"; then
    echo "ok   - $name"
  else
    echo "FAIL - $name"
    FAILS=$((FAILS + 1))
  fi
}

run_cli() {
  run_with_timeout 10 python3 -m logfilter "$@"
}

# 1. No filters at all: every line, exit 0.
out=$(run_cli "$LOG" 2>/dev/null); code=$?
check "no filters: exit 0, all 6 lines" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 6 ]'

# 2. --level filters, case-insensitively (lowercase input against uppercase log levels).
out=$(run_cli "$LOG" --level error 2>/dev/null); code=$?
check "level=error (lowercase) matches ERROR lines, exit 0" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 2 ]'

# 3. --level matches exact-case input too.
out=$(run_cli "$LOG" --level ERROR 2>/dev/null); code=$?
check "level=ERROR (uppercase) matches ERROR lines, exit 0" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 2 ]'

# 4. --level with mixed case.
out=$(run_cli "$LOG" --level WaRn 2>/dev/null); code=$?
check "level=WaRn (mixed case) matches WARN lines, exit 0" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 1 ]'

# 5. Unknown level is a usage error (exit 2), not silently accepted.
run_cli "$LOG" --level bogus >/dev/null 2>/tmp/logfilter_stderr_$$; code=$?
check "level=bogus: exit 2, stderr non-empty" bash -c '[ "'"$code"'" -eq 2 ] && [ -s /tmp/logfilter_stderr_'"$$"' ]'
rm -f "/tmp/logfilter_stderr_$$"

# 6. Bad --since date format is a usage error (exit 2), not silently ignored.
run_cli "$LOG" --since not-a-date >/dev/null 2>/tmp/logfilter_stderr_$$; code=$?
check "since=not-a-date: exit 2, stderr non-empty" bash -c '[ "'"$code"'" -eq 2 ] && [ -s /tmp/logfilter_stderr_'"$$"' ]'
rm -f "/tmp/logfilter_stderr_$$"

# 7. --since and --last together is a usage error (exit 2).
run_cli "$LOG" --since 2024-01-01 --last 2 >/dev/null 2>/tmp/logfilter_stderr_$$; code=$?
check "since+last together: exit 2, stderr non-empty" bash -c '[ "'"$code"'" -eq 2 ] && [ -s /tmp/logfilter_stderr_'"$$"' ]'
rm -f "/tmp/logfilter_stderr_$$"

# 8. --last 0 is a usage error (exit 2).
run_cli "$LOG" --last 0 >/dev/null 2>/tmp/logfilter_stderr_$$; code=$?
check "last=0: exit 2, stderr non-empty" bash -c '[ "'"$code"'" -eq 2 ] && [ -s /tmp/logfilter_stderr_'"$$"' ]'
rm -f "/tmp/logfilter_stderr_$$"

# 9. --last negative is a usage error (exit 2).
run_cli "$LOG" --last -3 >/dev/null 2>/tmp/logfilter_stderr_$$; code=$?
check "last=-3: exit 2, stderr non-empty" bash -c '[ "'"$code"'" -eq 2 ] && [ -s /tmp/logfilter_stderr_'"$$"' ]'
rm -f "/tmp/logfilter_stderr_$$"

# 10. Nonexistent log file is a runtime error (exit 1), distinct from usage errors.
run_cli "sample_logs/does-not-exist.log" >/dev/null 2>/tmp/logfilter_stderr_$$; code=$?
check "missing file: exit 1 (not 0, not 2), stderr non-empty" bash -c '[ "'"$code"'" -eq 1 ] && [ -s /tmp/logfilter_stderr_'"$$"' ]'
rm -f "/tmp/logfilter_stderr_$$"

# 11. --last N (valid) keeps only the last N matching lines, in original order.
out=$(run_cli "$LOG" --last 2 2>/dev/null); code=$?
check "last=2: exit 0, exactly the last 2 lines" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 2 ] && echo "'"$out"'" | grep -q "2024-01-15" && echo "'"$out"'" | grep -q "2024-01-14"'

# 12. --since (valid) keeps lines on/after that date.
out=$(run_cli "$LOG" --since 2024-01-13 2>/dev/null); code=$?
check "since=2024-01-13: exit 0, exactly 3 lines" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 3 ]'

# 13. --level and --since combined (AND semantics), a valid combination.
out=$(run_cli "$LOG" --level info --since 2024-01-14 2>/dev/null); code=$?
check "level=info + since=2024-01-14: exit 0, exactly 1 line" bash -c '[ "'"$code"'" -eq 0 ] && [ "$(echo "'"$out"'" | grep -c .)" -eq 1 ] && echo "'"$out"'" | grep -q "2024-01-14"'

# 14. A valid filter that matches nothing: exit 0, no output (not an error).
out=$(run_cli "$LOG" --level debug --since 2024-01-15 2>/dev/null); code=$?
check "no matches: exit 0, empty output" bash -c '[ "'"$code"'" -eq 0 ] && [ -z "'"$out"'" ]'

# 15. --help still works (argparse default), exit 0.
run_cli --help >/dev/null 2>/dev/null; code=$?
check "--help: exit 0" bash -c '[ "'"$code"'" -eq 0 ]'

echo "---"
if [ "$FAILS" -eq 0 ]; then
  echo "PASS harness-cli-argparse-log-filter"
  exit 0
else
  echo "FAIL harness-cli-argparse-log-filter ($FAILS check(s) failed)"
  exit 1
fi
