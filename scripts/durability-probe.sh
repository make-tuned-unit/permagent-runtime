#!/usr/bin/env bash
#
# durability-probe.sh — periodic durability metrics sampler (#560, item 6).
#
# WHY: "runs for weeks" is a time-domain property — a slow RSS leak, an fd creep,
# a WAL that never truncates, or a wedged scheduler job cannot be proven by a
# code read; they must be MEASURED over days. This is DURABILITY_AUDIT.md's
# "Part A external sampler": a tiny launchd agent that appends one JSON line per
# tick to a self-rotating metrics log, so the slow drift is visible after the
# fact. It writes to the SAME near-full disk it watches, so it is bounded (self-
# rotated at a size cap) and never becomes the disk-fill it is trying to catch.
#
# It is cheap by design: a handful of ps/lsof/stat/df calls every ~5 min, no
# daemon code, no busy-loop. Surface it later via the U1 daemon-health pill.
#
# Metrics per tick (all best-effort; missing values render as "?"):
#   pid, rss_kb, pcpu, etime, state   — process liveness + leak (F1/F8/F9);
#                                        a CHANGED pid = the daemon restarted
#   fd                                — open file descriptors (fd leak)
#   tcp_est                           — established conns on the daemon port
#                                        (/events socket leak)
#   memdb_wal_kb, specdb_wal_kb       — SQLite WAL sizes (F4 checkpoint wedge)
#   disk_free_kb                      — Data volume free space (F5/F11 disk fill)
#   daemon_err_kb                     — daemon.err size (F5 log growth rate)
#   llm_orphans                       — llm_request.*.jsonl orphan count (F10)
#   crashes, crash_newest             — crash-file count + newest mtime; a NEW
#                                        crash with an UNCHANGED pid ⇒ F1 half-dead
#   sched_running, sched_min_last_run_age_s — wedged-job signal (F2)
#
# Install as a launchd agent with scripts/ai.permagent.durability-probe.plist.
#
# Usage:
#   scripts/durability-probe.sh          # run forever (launchd/foreground)
#   TICK=300 scripts/durability-probe.sh # override tick seconds (default 300)
#   scripts/durability-probe.sh --once   # single sample to stdout+file, then exit
#
set -uo pipefail

TICK="${TICK:-300}"                                   # 5 minutes
LOG_DIR="${PERMAGENT_LOG_DIR:-$HOME/.permagent/logs}"
STATE_DIR="${PERMAGENT_STATE_DIR:-$HOME/.permagent}"
PROBE="$LOG_DIR/durability-probe.jsonl"
PROBE_MAX_BYTES="${PROBE_MAX_BYTES:-5242880}"         # 5 MiB, self-rotated
DAEMON_PORT="${DAEMON_PORT:-3001}"
DAEMON_PATTERN="${DAEMON_PATTERN:-Permagent.app/Contents/MacOS/permagentd}"

MEMDB_WAL="$STATE_DIR/brain/memory.db-wal"
SPECDB_WAL="$STATE_DIR/spectral/permagent.db-wal"
DAEMON_ERR="$LOG_DIR/daemon.err"
CRASH_DIR="$STATE_DIR/crashes"
SCHEDULE_JSON="$STATE_DIR/schedule.json"

mkdir -p "$LOG_DIR"

log() { printf '[durability-probe %s] %s\n' "$(date '+%H:%M:%S')" "$*" >&2; }

# Bytes of a file (0 if absent), rendered in KiB.
size_kb() {
  local f="$1" sz
  [ -f "$f" ] || { echo 0; return; }
  sz=$(stat -f%z "$f" 2>/dev/null || echo 0)
  echo $(( sz / 1024 ))
}

rotate_probe() {
  [ -f "$PROBE" ] || return 0
  local sz
  sz=$(stat -f%z "$PROBE" 2>/dev/null || echo 0)
  if [ "$sz" -gt "$PROBE_MAX_BYTES" ]; then
    mv -f "$PROBE" "$PROBE.1"
  fi
}

daemon_pid() {
  pgrep -f "$DAEMON_PATTERN" 2>/dev/null | head -1
}

# Parse schedule.json for the wedged-job signal (F2): how many jobs are flagged
# currently_running, and the smallest last_run age in seconds (a running job
# whose last_run is far older than its interval is wedged). Uses python3 when
# present for a robust parse; degrades to "?" otherwise.
scheduler_metrics() {
  if [ ! -f "$SCHEDULE_JSON" ] || ! command -v python3 >/dev/null 2>&1; then
    echo "? ?"; return
  fi
  python3 - "$SCHEDULE_JSON" <<'PY' 2>/dev/null || echo "? ?"
import json, sys, datetime
try:
    data = json.load(open(sys.argv[1]))
except Exception:
    print("? ?"); sys.exit(0)
jobs = data if isinstance(data, list) else data.get("jobs", [])
running = 0
min_age = None
now = datetime.datetime.now(datetime.timezone.utc)
for j in jobs if isinstance(jobs, list) else []:
    if not isinstance(j, dict):
        continue
    if j.get("currently_running"):
        running += 1
    lr = j.get("last_run")
    if lr:
        try:
            dt = datetime.datetime.fromisoformat(str(lr).replace("Z", "+00:00"))
            if dt.tzinfo is None:
                dt = dt.replace(tzinfo=datetime.timezone.utc)
            age = int((now - dt).total_seconds())
            min_age = age if min_age is None else min(min_age, age)
        except Exception:
            pass
print(f"{running} {min_age if min_age is not None else '?'}")
PY
}

sample_line() {
  local ts pid psline pcpu rss state etime fd tcp
  ts=$(date '+%Y-%m-%dT%H:%M:%S%z')
  pid=$(daemon_pid)

  local disk_free crashes crash_newest llm_orphans sched
  disk_free=$(df -k /System/Volumes/Data 2>/dev/null | awk 'NR==2{print $4}')
  crashes=$(ls -1 "$CRASH_DIR" 2>/dev/null | wc -l | tr -d ' ')
  crash_newest=$(ls -t "$CRASH_DIR" 2>/dev/null | head -1)
  llm_orphans=$(ls -1 "$LOG_DIR"/llm_request.*.jsonl 2>/dev/null | wc -l | tr -d ' ')
  sched=$(scheduler_metrics)
  local sched_running sched_age
  sched_running=$(awk '{print $1}' <<<"$sched")
  sched_age=$(awk '{print $2}' <<<"$sched")

  if [ -z "$pid" ]; then
    printf '{"ts":"%s","daemon":"absent","disk_free_kb":%s,"memdb_wal_kb":%s,"specdb_wal_kb":%s,"daemon_err_kb":%s,"llm_orphans":%s,"crashes":%s,"crash_newest":"%s","sched_running":"%s","sched_min_last_run_age_s":"%s"}\n' \
      "$ts" "${disk_free:-0}" "$(size_kb "$MEMDB_WAL")" "$(size_kb "$SPECDB_WAL")" \
      "$(size_kb "$DAEMON_ERR")" "${llm_orphans:-0}" "${crashes:-0}" "${crash_newest:-none}" \
      "${sched_running:-?}" "${sched_age:-?}"
    return
  fi

  psline=$(ps -o pcpu=,rss=,state=,etime= -p "$pid" 2>/dev/null)
  pcpu=$(awk '{print $1}' <<<"$psline"); rss=$(awk '{print $2}' <<<"$psline")
  state=$(awk '{print $3}' <<<"$psline"); etime=$(awk '{print $4}' <<<"$psline")
  fd=$(lsof -p "$pid" 2>/dev/null | wc -l | tr -d ' ')
  tcp=$(lsof -nP -iTCP:"$DAEMON_PORT" 2>/dev/null | grep -c ESTABLISHED)

  printf '{"ts":"%s","pid":%s,"rss_kb":"%s","pcpu":"%s","state":"%s","etime":"%s","fd":%s,"tcp_est":%s,"memdb_wal_kb":%s,"specdb_wal_kb":%s,"disk_free_kb":%s,"daemon_err_kb":%s,"llm_orphans":%s,"crashes":%s,"crash_newest":"%s","sched_running":"%s","sched_min_last_run_age_s":"%s"}\n' \
    "$ts" "$pid" "${rss:-?}" "${pcpu:-?}" "${state:-?}" "${etime:-?}" "${fd:-0}" "${tcp:-0}" \
    "$(size_kb "$MEMDB_WAL")" "$(size_kb "$SPECDB_WAL")" "${disk_free:-0}" "$(size_kb "$DAEMON_ERR")" \
    "${llm_orphans:-0}" "${crashes:-0}" "${crash_newest:-none}" "${sched_running:-?}" "${sched_age:-?}"
}

tick() {
  rotate_probe
  local line; line=$(sample_line)
  printf '%s\n' "$line" >> "$PROBE"
  printf '%s\n' "$line"
}

case "${1:-}" in
  --once) tick; exit 0 ;;
esac

log "started (tick=${TICK}s, probe=$PROBE)"
while true; do
  tick >/dev/null
  sleep "$TICK"
done
