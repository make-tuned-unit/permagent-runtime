#!/bin/bash
# Parameterized SINGLE pooled cell. Runs ON the M4 (head); M1 = RPC worker (MTL0).
# Usage: pod_cell.sh <label> <model.gguf> <ts> <ctx> <ngen> [wd_thresh_mb]
# Safety: local M4 watchdog (1s spike log + kill-before-panic) + 90s warmup timeout.
# Default-wired-limit assumed (do NOT raise to 13000 — that turns OOM into panic).
set -u
BIN=~/dev/mesh-spike/bin
MODELS=~/dev/mesh-spike/models
RPC=169.254.233.101:50052
WIRED_IF=en0
PROMPT=~/dev/mesh-spike/bench/prompt.txt
CLIENT=~/dev/mesh-spike/bench/ttft_client.py
WD=~/dev/mesh-spike/bench/m4_watchdog.sh
PORT=8080

label="$1"; file="$2"; ts="$3"; ctx="$4"; ngen="$5"; wdth="${6:-3500}"
wdlog=/tmp/m4_wd_$label.log
srvlog=/tmp/srv_$label.log

echo "==================== POOLED CELL $label ===================="
echo "model=$file ts=$ts ctx=$ctx ngen=$ngen wd_thresh=${wdth}MB  START $(date +%H:%M:%S)"
echo "M4 wired_limit (must be default/0): $(sysctl -n iogpu.wired_limit_mb)"

# launch local watchdog
bash "$WD" "$wdth" "$wdlog" &
wdpid=$!

rx0=$(netstat -ibn -I $WIRED_IF | awk 'NR>1 && $1=="'$WIRED_IF'"{print $7; exit}')
"$BIN/llama-server" -m "$MODELS/$file" --rpc "$RPC" -ngl 99 -ts "$ts" \
     --host 127.0.0.1 --port $PORT -c "$ctx" --no-webui > "$srvlog" 2>&1 &
srv=$!

# 90s HARD warmup timeout
ok=0
for i in $(seq 1 90); do
  if curl -sf http://127.0.0.1:$PORT/health 2>/dev/null | grep -q '"ok"'; then ok=1; break; fi
  if ! kill -0 $srv 2>/dev/null; then echo "RESULT: SERVER_DIED_DURING_LOAD at ${i}s"; break; fi
  sleep 1
done

if [ $ok -eq 1 ]; then
  echo "LOADED $(date +%H:%M:%S) — running 3 trials"
  python3 "$CLIENT" "http://127.0.0.1:$PORT" "$PROMPT" "$ngen" 3
  echo "SERVER_TIMINGS:"; grep -iE "prompt eval time|^.*eval time|tokens per second" "$srvlog" | tail -6
else
  if kill -0 $srv 2>/dev/null; then echo "RESULT: WARMUP_TIMEOUT_90s (killing)"; else echo "RESULT: did not reach healthy"; fi
fi

rx1=$(netstat -ibn -I $WIRED_IF | awk 'NR>1 && $1=="'$WIRED_IF'"{print $7; exit}')
echo "WIRED_RX_DELTA_BYTES $((rx1-rx0))"
# classify failure mode from logs
grep -iE "OutOfMemory|Compute error" "$srvlog" >/dev/null 2>&1 && echo "FAILMODE: GRACEFUL_GPU_OOM (acceptable)"
grep -iE "WATCHDOG_TRIP" "$wdlog" 2>/dev/null && echo "FAILMODE: WATCHDOG_KILLED (avail<thresh — prevented panic)"
# warmup spike summary
echo "--- M4 warmup memory trace (min avail, peak wired) ---"
awk 'NR>1{print}' "$wdlog" | awk '{if(min==""||$8<min)min=$8; if($4>pw)pw=$4} END{print "min_avail="min"MB  peak_wired="pw"MB"}'
echo "--- last 8 trace rows ---"; tail -8 "$wdlog"

kill $srv 2>/dev/null; wait $srv 2>/dev/null
kill $wdpid 2>/dev/null
echo "DONE $label $(date +%H:%M:%S)"
