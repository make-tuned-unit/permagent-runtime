#!/bin/bash
# M4-LOCAL memory watchdog + warmup spike logger. Runs ON the M4 (head).
# Logs free/inactive/wired/compressed/swap/llama-server-RSS every 1s so we
# capture the SHAPE of the warmup transient that panicked the box at ~14GB free.
# Kills llama-server locally (fastest possible abort) if available RAM drops
# below THRESH — fires BEFORE a panic, not after (the SSH-polled M1 guard lagged).
THRESH_MB=${1:-3500}
LOG=${2:-/tmp/m4_watchdog.log}
PGSZ=16384
echo "# ts free_mb inact_mb wired_mb comp_mb swap_mb srv_rss_mb avail_mb" > "$LOG"
while true; do
  vals=$(vm_stat | awk -v p=$PGSZ '
    /Pages free/{gsub(/\./,"",$3); f=$3}
    /Pages inactive/{gsub(/\./,"",$3); i=$3}
    /Pages wired down/{gsub(/\./,"",$4); w=$4}
    /occupied by compressor/{gsub(/\./,"",$5); c=$5}
    END{printf "%d %d %d %d", f*p/1048576, i*p/1048576, w*p/1048576, c*p/1048576}')
  free_mb=$(echo $vals | cut -d' ' -f1); inact_mb=$(echo $vals | cut -d' ' -f2)
  wired_mb=$(echo $vals | cut -d' ' -f3); comp_mb=$(echo $vals | cut -d' ' -f4)
  swap_mb=$(sysctl -n vm.swapusage | awk '{gsub(/M/,"",$6); print int($6)}')
  pid=$(pgrep -f "bin/llama-server" | head -1)
  rss=0; [ -n "$pid" ] && rss=$(ps -o rss= -p $pid 2>/dev/null | awk '{print int($1/1024)}')
  avail=$((free_mb+inact_mb))
  echo "$(date +%H:%M:%S) $free_mb $inact_mb $wired_mb $comp_mb $swap_mb $rss $avail" >> "$LOG"
  if [ "$avail" -lt "$THRESH_MB" ] && [ -n "$pid" ]; then
    echo "$(date +%H:%M:%S) WATCHDOG_TRIP avail=${avail}MB < ${THRESH_MB}MB -> kill -9 llama-server $pid" >> "$LOG"
    kill -9 "$pid"
    break
  fi
  sleep 1
done
