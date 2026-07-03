#!/usr/bin/env bash
#
# daemon-health-watchdog.sh — restart permagentd when it is ALIVE BUT WEDGED
# (durability #560, item 2).
#
# WHY: launchd's KeepAlive only relaunches on process *exit*. A daemon whose
# process is alive but no longer answering HTTP (a deadlocked runtime, an
# exhausted connection pool, a blocked accept loop) is invisible to launchd and
# will never be restarted — the exact failure the "reachable-for-weeks" bar
# cannot tolerate. This external probe hits the daemon's public /status endpoint
# and, after N consecutive unanswered ticks, forces a clean launchd restart via
# `launchctl kickstart -k`. It mirrors the UI-wedge watchdog (#562/#563).
#
# It does NOT catch the "half-dead" case (a background task died but HTTP still
# answers) — that is covered by the in-process panic circuit-breaker (item 1)
# and the "new crash-file, same PID" heuristic in the durability probe (item 6).
# This watchdog is specifically for the FULL wedge that still holds the socket.
#
# WHAT each tick (default 15 s):
#   1. curl the daemon /status with a short timeout.
#   2. Reachable (any real HTTP code) -> healthy, re-arm.
#   3. Unreachable ("000": refused or timed out) N consecutive times -> capture a
#      `sample` of the wedged daemon (money shot) + a process snapshot, then
#      `launchctl kickstart -k` the daemon. Fire once per episode; re-arm on
#      recovery or PID change.
#
# Install as a launchd agent with scripts/ai.permagent.daemon-watchdog.plist.
#
# Usage:
#   scripts/daemon-health-watchdog.sh            # run forever (launchd/foreground)
#   TICK=10 scripts/daemon-health-watchdog.sh    # override tick seconds
#   scripts/daemon-health-watchdog.sh --once     # single tick, then exit (testing)
#   RESTART=0 scripts/daemon-health-watchdog.sh  # capture-only, never restart
#
set -uo pipefail

TICK="${TICK:-15}"
LOG_DIR="${PERMAGENT_LOG_DIR:-$HOME/.permagent/logs}"
DAEMON_PORT="${DAEMON_PORT:-3001}"
# /status is PUBLIC (no Bearer) and returns 200 "ok" — a safe liveness target
# that leaks nothing (no query tokens, no data).
DAEMON_URL="${DAEMON_URL:-http://127.0.0.1:${DAEMON_PORT}/status}"
CURL_TIMEOUT="${CURL_TIMEOUT:-5}"
# Consecutive unreachable ticks before we declare a wedge and restart.
FAIL_THRESHOLD="${FAIL_THRESHOLD:-3}"
# launchd service target for `launchctl kickstart`.
DAEMON_LABEL="${DAEMON_LABEL:-ai.permagent.daemon}"
# Match the daemon process for `sample` capture. Excludes this script.
DAEMON_PATTERN="${DAEMON_PATTERN:-Permagent.app/Contents/MacOS/permagentd}"
# Set RESTART=0 to make the watchdog capture-only (never kickstart).
RESTART="${RESTART:-1}"

SAMPLE="${SAMPLE:-/usr/bin/sample}"

mkdir -p "$LOG_DIR"

log() { printf '[daemon-watchdog %s] %s\n' "$(date '+%H:%M:%S')" "$*" >&2; }

daemon_pid() {
  pgrep -f "$DAEMON_PATTERN" 2>/dev/null | head -1
}

# Reachability probe. Echoes the HTTP status code, or "000" when the connection
# failed / timed out (the wedge signal).
http_code() {
  curl -m "$CURL_TIMEOUT" -s -o /dev/null -w '%{http_code}' "$DAEMON_URL" 2>/dev/null || echo 000
}

# Capture what the wedged daemon is stuck on, then a process snapshot.
capture() {
  local pid="$1" ts dir
  ts=$(date '+%Y%m%d-%H%M%S')
  dir="$LOG_DIR/daemon-wedge-$ts"
  mkdir -p "$dir"
  log "WEDGE — capturing evidence to $dir (daemon pid ${pid:-none})"
  if [ -n "$pid" ]; then
    "$SAMPLE" "$pid" 10 -mayDie -file "$dir/daemon-sample.txt" >/dev/null 2>&1 \
      || echo "sample unavailable for pid $pid" > "$dir/daemon-sample.txt"
  else
    echo "daemon process not found by pattern '$DAEMON_PATTERN'" > "$dir/daemon-sample.txt"
  fi
  {
    printf 'url=%s code=000 (unreachable)\n' "$DAEMON_URL"
    printf 'threshold=%s consecutive ticks\n' "$FAIL_THRESHOLD"
  } > "$dir/health.txt"
  ps -Ao pid,ppid,pcpu,rss,state,etime,comm 2>/dev/null | grep -i 'permagent' \
    > "$dir/process-snapshot.txt"
  echo "$dir"
}

# Force a clean launchd restart. `kickstart -k` kills the current instance and
# relaunches it under launchd (so KeepAlive/ThrottleInterval still apply).
restart_daemon() {
  local dir="$1"
  if [ "$RESTART" != "1" ]; then
    log "RESTART=0 — capture-only, leaving wedged daemon in place"
    return 0
  fi
  local target="gui/$(id -u)/${DAEMON_LABEL}"
  log "restarting wedged daemon via: launchctl kickstart -k $target"
  if launchctl kickstart -k "$target" 2>>"$dir/restart.txt"; then
    log "kickstart issued for $target"
    echo "restarted: $(date '+%Y-%m-%dT%H:%M:%S%z') via kickstart -k $target" >> "$dir/restart.txt"
  else
    log "kickstart FAILED for $target — see $dir/restart.txt"
  fi
}

CONSEC=0          # consecutive unreachable ticks
ARMED=1           # 1 = ready to restart; 0 = already restarted this episode
LAST_PID=""

tick() {
  local pid code
  pid=$(daemon_pid)
  # A new daemon instance (a restart happened) re-arms the detector.
  if [ "$pid" != "$LAST_PID" ]; then
    LAST_PID="$pid"; CONSEC=0; ARMED=1
  fi

  code=$(http_code)
  if [ "$code" = "000" ]; then
    CONSEC=$((CONSEC + 1))
    log "daemon UNREACHABLE ($DAEMON_URL) — $CONSEC/$FAIL_THRESHOLD consecutive (pid ${pid:-none})"
    if [ "$CONSEC" -ge "$FAIL_THRESHOLD" ] && [ "$ARMED" -eq 1 ]; then
      local dir; dir=$(capture "$pid")
      restart_daemon "$dir"
      ARMED=0     # one restart per episode
    fi
  else
    if [ "$CONSEC" -gt 0 ]; then log "daemon recovered (HTTP $code) — re-armed"; fi
    CONSEC=0; ARMED=1
  fi
}

case "${1:-}" in
  --once) tick; exit 0 ;;
esac

log "started (tick=${TICK}s, url=$DAEMON_URL, threshold=$FAIL_THRESHOLD, restart=$RESTART)"
while true; do
  tick
  sleep "$TICK"
done
