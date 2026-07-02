#!/usr/bin/env bash
#
# ui-wedge-watchdog.sh — capture the wedged main-thread stack of the Permagent
# desktop UI process when it hangs after idle (#562).
#
# WHY: the UI app main thread blocks after ~1 h idle (window won't drag, panes
# blank) while `permagentd` stays healthy. The wedge is unattended and the app
# is unresponsive when it happens, so the root-cause stack must be captured
# automatically and EXTERNALLY (zero UI-process code — cannot add to the hang).
# The captured thread-0 stack disambiguates the ranked candidates in
# docs/architecture/UI_IDLE_WEDGE_DIAGNOSIS.md (C1/C2 WebKit sync-IPC vs. C4
# Rust mutex vs. a pure App-Nap suspension).
#
# WHAT it does each tick (default 30 s):
#   1. Locate the GUI process + its WebContent children; append one self-rotating
#      JSONL probe line (ps/fd/tcp) to ~/.permagent/logs/ui-durability-probe.jsonl
#   2. Wedge test: PRIMARY = the app's main-thread heartbeat file has gone stale
#      (main.rs stamps ~/.permagent/logs/ui-heartbeat every 5s from the main
#      thread; stale => the run loop is not draining). FALLBACK (older builds w/o
#      the heartbeat) = `sample <pid> 1` shows thread 0 in a NON-idle wait. Two
#      consecutive wedged ticks -> declare a wedge.
#   3. On wedge, capture ~/.permagent/logs/ui-wedge-<ts>/:
#        - sample <gui-pid> 10           (money shot: main-thread stack)
#        - spindump <gui-pid> 10         (best-effort; needs privileges)
#        - sample of each WebContent child
#        - daemon health curl            (auto re-confirms the #562 boundary)
#        - tail of the probe JSONL        (fd/conn trend into the wedge)
#   4. On wedge, after capture, RECOVER: kill the wedged GUI + relaunch it
#      (RELAUNCH=1 default; the daemon is never touched). Set RELAUNCH=0 for
#      capture-only. This is the UI-side of #560's fail-fast-recover.
#   5. Fire once per episode; re-arm when the main thread returns to idle or the
#      pid changes.
#
# Ships FIRST, mirroring DURABILITY_AUDIT.md (#560) "Part A external sampler".
# Install as a launchd agent with scripts/ai.permagent.ui-watchdog.plist.
#
# Usage:
#   scripts/ui-wedge-watchdog.sh              # run forever (launchd / foreground)
#   TICK=15 scripts/ui-wedge-watchdog.sh      # override tick seconds
#   scripts/ui-wedge-watchdog.sh --once       # single tick, then exit (testing)
#   scripts/ui-wedge-watchdog.sh --capture    # force a capture now, then exit
#
set -uo pipefail

TICK="${TICK:-30}"
LOG_DIR="${PERMAGENT_LOG_DIR:-$HOME/.permagent/logs}"
PROBE="$LOG_DIR/ui-durability-probe.jsonl"
PROBE_MAX_BYTES="${PROBE_MAX_BYTES:-5242880}"   # 5 MiB, self-rotated (never F5)
DAEMON_URL="${DAEMON_URL:-http://127.0.0.1:3001/api/people}"
# GUI process match: the app bundle's MacOS binary, NOT permagentd, NOT this script.
GUI_PATTERN="${GUI_PATTERN:-Permagent.app/Contents/MacOS/permagent-app}"
# Main-thread heartbeat file stamped by the app (main.rs start_main_thread_heartbeat,
# #562). When present it is the PRIMARY, crisp wedge signal; the sample heuristic
# is the fallback for app builds that predate it. Stale past this many seconds
# (with the process alive) = the main run loop is not draining = wedged.
HEARTBEAT="${HEARTBEAT:-$LOG_DIR/ui-heartbeat}"
HEARTBEAT_STALE_SECS="${HEARTBEAT_STALE_SECS:-20}"   # app stamps every 5s
# Auto-recover: after capturing evidence, kill the wedged GUI and relaunch it.
# This is the UI-side of #560's fail-fast-recover (the daemon has launchd;
# the UI gets the watchdog). The daemon is NEVER touched. Set RELAUNCH=0 to
# make the watchdog capture-only.
RELAUNCH="${RELAUNCH:-1}"
APP_BUNDLE="${APP_BUNDLE:-/Applications/Permagent.app}"

# Absolute paths — a Python package on PATH shadows `sample` with a broken
# stub, and `spindump` lives in /usr/sbin. Never rely on PATH resolution here.
SAMPLE="${SAMPLE:-/usr/bin/sample}"
SPINDUMP="${SPINDUMP:-/usr/sbin/spindump}"

mkdir -p "$LOG_DIR"

# ---- helpers ---------------------------------------------------------------

log() { printf '[ui-watchdog %s] %s\n' "$(date '+%H:%M:%S')" "$*" >&2; }

# PID of the GUI process (empty if not running). Excludes grep/self.
gui_pid() {
  pgrep -f "$GUI_PATTERN" 2>/dev/null | head -1
}

# WebContent child pids (the WKWebView content processes: main webview + browser).
webcontent_pids() {
  pgrep -f "Permagent.*WebContent\|com.apple.WebKit.WebContent" 2>/dev/null
}

# Rotate the probe file if it exceeds the cap (keep one .1 backup).
rotate_probe() {
  [ -f "$PROBE" ] || return 0
  local sz
  sz=$(stat -f%z "$PROBE" 2>/dev/null || echo 0)
  if [ "$sz" -gt "$PROBE_MAX_BYTES" ]; then
    mv -f "$PROBE" "$PROBE.1"
  fi
}

# One JSONL probe line for the GUI process.
probe_line() {
  local pid="$1" ts fd tcp psline pcpu rss state etime
  ts=$(date '+%Y-%m-%dT%H:%M:%S%z')
  if [ -z "$pid" ]; then
    printf '{"ts":"%s","gui":"absent"}\n' "$ts"
    return
  fi
  psline=$(ps -o pcpu=,rss=,state=,etime= -p "$pid" 2>/dev/null)
  pcpu=$(awk '{print $1}' <<<"$psline"); rss=$(awk '{print $2}' <<<"$psline")
  state=$(awk '{print $3}' <<<"$psline"); etime=$(awk '{print $4}' <<<"$psline")
  fd=$(lsof -p "$pid" 2>/dev/null | wc -l | tr -d ' ')
  tcp=$(lsof -nP -p "$pid" -iTCP 2>/dev/null | grep -c ESTABLISHED)
  printf '{"ts":"%s","pid":%s,"pcpu":"%s","rss_kb":"%s","state":"%s","etime":"%s","fd":%s,"tcp_est":%s}\n' \
    "$ts" "$pid" "${pcpu:-?}" "${rss:-?}" "${state:-?}" "${etime:-?}" "${fd:-0}" "${tcp:-0}"
}

# Heartbeat age in seconds, or empty if the file is absent (older app build).
heartbeat_age() {
  [ -f "$HEARTBEAT" ] || return 1
  local mtime now
  mtime=$(stat -f%m "$HEARTBEAT" 2>/dev/null) || return 1
  now=$(date +%s)
  echo $(( now - mtime ))
}

# Primary wedge signal: the main-thread heartbeat has gone stale. Returns 0 when
# the heartbeat exists AND is older than the threshold (main run loop not
# draining). Returns 1 when fresh; returns 2 when the file is absent (caller
# should fall back to the sample heuristic).
heartbeat_wedged() {
  local age; age=$(heartbeat_age) || return 2
  [ "$age" -gt "$HEARTBEAT_STALE_SECS" ] && return 0 || return 1
}

# Is the GUI main thread (thread 0) in a NON-idle wait? Returns 0 (wedged-ish)
# when the sampled thread-0 stack is a real block, 1 when it looks like the
# benign CFRunLoop event wait (normal idle) or we can't tell. This is the
# FALLBACK detector for app builds without the heartbeat.
#
# A healthy idle Cocoa main thread sits in mach_msg_trap under
# __CFRunLoopServiceMachPort / __CFRunLoopRun. A wedge that will not drag shows a
# DIFFERENT frame on top of the mach_msg wait (WebKit sync IPC, a mutex, etc.).
main_thread_wedged() {
  local pid="$1" s t0
  s=$("$SAMPLE" "$pid" 1 -mayDie 2>/dev/null) || return 1
  # Isolate the main-thread (`com.apple.main-thread`) block of the call graph,
  # from its header line to the next `Thread_` header. macOS `sample` prints an
  # indented call tree under `Call graph:`.
  t0=$(awk '/com\.apple\.main-thread/{f=1}
            f && /^ +[0-9]+ Thread_/ && !/main-thread/ {exit}
            f {print}' <<<"$s")
  [ -n "$t0" ] || return 1
  # Idle heuristic: a Cocoa main thread parked in its run-loop event wait is
  # healthy, NOT wedged.
  if grep -Eq 'CFRunLoopServiceMachPort|__CFRunLoopRun|ReceiveNextEventCommon|_DPSNextEvent|NSApplication.*run' <<<"$t0"; then
    return 1
  fi
  # Real-block heuristic: a distinctive blocking frame on the main thread
  # (WebKit sync IPC -> C1/C2; psync mutex -> C4).
  if grep -Eq '__psynch_mutexwait|__psynch_cvwait|semaphore_wait|WebPageProxy|IPC::Connection|sendSync|WebKit::' <<<"$t0"; then
    return 0
  fi
  # A main thread NOT in the benign run-loop wait is suspicious: flag it and let
  # the two-consecutive-tick gate + the full capture adjudicate.
  return 0
}

# Full evidence capture for a confirmed wedge.
capture() {
  local pid="$1" ts dir
  ts=$(date '+%Y%m%d-%H%M%S')
  dir="$LOG_DIR/ui-wedge-$ts"
  mkdir -p "$dir"
  log "WEDGE — capturing evidence to $dir (gui pid $pid)"

  "$SAMPLE" "$pid" 10 -mayDie -file "$dir/gui-sample.txt" >/dev/null 2>&1 &
  # spindump needs privileges; capture best-effort and note if denied.
  ( "$SPINDUMP" "$pid" 10 -o "$dir/gui-spindump.txt" >/dev/null 2>&1 \
      || echo "spindump unavailable (needs sudo / Developer Tools rights)" \
         > "$dir/gui-spindump.txt" ) &

  # WebContent children — is the content process also blocked (C1/C2) or fine?
  local wc
  for wc in $(webcontent_pids); do
    "$SAMPLE" "$wc" 5 -mayDie -file "$dir/webcontent-$wc-sample.txt" >/dev/null 2>&1 &
  done

  # Auto re-confirm the #562 boundary: is the daemon still serving during the
  # wedge? ANY HTTP code (200, 401, ...) means the daemon is alive and answering;
  # "000" means the connection failed (daemon down / unreachable).
  {
    local code
    code=$(curl -m 3 -s -o /dev/null -w '%{http_code}' "$DAEMON_URL" 2>/dev/null || echo 000)
    printf 'daemon %s -> HTTP %s\n' "$DAEMON_URL" "$code"
    if [ "$code" = "000" ]; then
      echo 'verdict: daemon UNREACHABLE during wedge (daemon-level block — see #560)'
    else
      echo 'verdict: daemon ALIVE during wedge (UI-process-only block — remote bar intact)'
    fi
  } > "$dir/daemon-health.txt"

  # Trend into the wedge.
  tail -n 40 "$PROBE" 2>/dev/null > "$dir/probe-tail.jsonl"
  ps -Ao pid,ppid,pcpu,rss,state,comm 2>/dev/null | grep -iE 'permagent|WebContent' \
    > "$dir/process-snapshot.txt"

  wait
  log "capture complete: $dir"
  echo "$dir"
}

# Recover the wedged GUI: kill it and relaunch. NEVER touches permagentd — the
# daemon is healthy during the wedge (that is the whole #562 finding) and has
# its own launchd supervision. Idempotent and best-effort.
relaunch_gui() {
  local pid="$1" dir="$2"
  if [ "$RELAUNCH" != "1" ]; then
    log "RELAUNCH=0 — capture-only, leaving wedged GUI (pid $pid) in place"
    return 0
  fi
  log "recovering: killing wedged GUI pid $pid (daemon untouched) + relaunching"
  # Target the GUI binary ONLY (pattern excludes permagentd); escalate to -9.
  pkill -f "$GUI_PATTERN" 2>/dev/null
  sleep 2
  pkill -9 -f "$GUI_PATTERN" 2>/dev/null
  sleep 1
  if [ -d "$APP_BUNDLE" ]; then
    open "$APP_BUNDLE" 2>/dev/null && log "relaunched $APP_BUNDLE"
  else
    log "APP_BUNDLE not found ($APP_BUNDLE) — relaunch skipped; killed only"
  fi
  echo "relaunched: $(date '+%Y-%m-%dT%H:%M:%S%z') (RELAUNCH=$RELAUNCH, bundle=$APP_BUNDLE)" \
    >> "$dir/recovery.txt"
}

# ---- main tick -------------------------------------------------------------

CONSEC=0          # consecutive wedged ticks
ARMED=1           # 1 = ready to capture; 0 = already captured this episode
LAST_PID=""

tick() {
  rotate_probe
  local pid; pid=$(gui_pid)
  probe_line "$pid" >> "$PROBE"

  if [ -z "$pid" ]; then
    CONSEC=0; ARMED=1; LAST_PID=""
    return
  fi
  # New process instance (relaunch) re-arms the detector.
  if [ "$pid" != "$LAST_PID" ]; then
    LAST_PID="$pid"; CONSEC=0; ARMED=1
  fi

  # Primary signal: stale main-thread heartbeat. Fall back to the sample
  # heuristic when the app predates the heartbeat (heartbeat_wedged -> 2).
  local wedged sig
  heartbeat_wedged; local hb=$?
  if [ "$hb" -eq 0 ]; then
    wedged=0; sig="heartbeat stale >${HEARTBEAT_STALE_SECS}s"
  elif [ "$hb" -eq 1 ]; then
    wedged=1; sig="heartbeat fresh"
  elif main_thread_wedged "$pid"; then
    wedged=0; sig="main-thread non-idle wait (no heartbeat)"
  else
    wedged=1; sig="idle (no heartbeat)"
  fi

  if [ "$wedged" -eq 0 ]; then
    CONSEC=$((CONSEC + 1))
    log "wedge signal: $sig ($CONSEC consecutive) pid $pid"
    if [ "$CONSEC" -ge 2 ] && [ "$ARMED" -eq 1 ]; then
      local dir; dir=$(capture "$pid")
      relaunch_gui "$pid" "$dir"
      ARMED=0     # one capture+recover per episode
    fi
  else
    if [ "$CONSEC" -gt 0 ]; then log "recovered ($sig) — re-armed"; fi
    CONSEC=0; ARMED=1
  fi
}

case "${1:-}" in
  --once)     tick; exit 0 ;;
  --capture)  p=$(gui_pid); [ -n "$p" ] && capture "$p" || { log "GUI not running"; exit 1; }; exit 0 ;;
esac

log "started (tick=${TICK}s, gui_pattern='$GUI_PATTERN', probe=$PROBE)"
while true; do
  tick
  sleep "$TICK"
done
