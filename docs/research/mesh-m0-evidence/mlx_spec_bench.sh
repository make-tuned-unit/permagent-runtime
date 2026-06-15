#!/bin/bash
# M0.5 — MLX speculative-decoding anchor bench. Runs ON the M4. Single-node, no RPC.
# Same prompt (~500 tok) + 256-gen as M0 controls, for apples-to-apples vs llama.cpp.
GEN=~/dev/mesh-spike/mlxenv/bin/mlx_lm.generate
PROMPT="$(cat ~/dev/mesh-spike/bench/prompt.txt)"
DRAFT=mlx-community/Qwen3-0.6B-4bit
RAMLOG=/tmp/mlx_ram.log
MAXTOK=256

# background RAM logger
( echo "# ts free_mb wired_mb"; while true; do
    vm_stat | awk -v t="$(date +%H:%M:%S)" '/Pages free/{gsub(/\./,"",$3);f=$3}/Pages wired/{gsub(/\./,"",$4);w=$4}END{printf "%s %d %d\n", t, f*16384/1048576, w*16384/1048576}'
    sleep 2
  done ) > "$RAMLOG" 2>&1 &
RAMPID=$!

run () {
  local label="$1"; shift
  echo "################## $label  $(date +%H:%M:%S) ##################"
  "$GEN" "$@" --prompt "$PROMPT" --max-tokens $MAXTOK --temp 0 2>&1 | grep -iE "Prompt:|Generation:|Peak memory:|tokens-per-sec|Error|error|Killed"
  echo "------ peak wired during (MB): $(awk 'NR>1{if($3>m)m=$3}END{print m}' $RAMLOG)  min free (MB): $(awk 'NR>1{if(min==""||$2<min)min=$2}END{print min}' $RAMLOG)"
  : > "$RAMLOG"; echo "# reset" >> "$RAMLOG"
  echo
}

echo "=== M0.5 MLX-spec anchor bench on M4 ($(sysctl -n hw.memsize | awk '{print $1/1073741824}')GB) ==="
run "14B-4bit BASELINE (no spec)"      --model mlx-community/Qwen3-14B-4bit
run "14B-4bit SPEC (draft 0.6B, k=4)"  --model mlx-community/Qwen3-14B-4bit --draft-model $DRAFT --num-draft-tokens 4
run "30B-A3B-3bit BASELINE (no spec)"  --model mlx-community/Qwen3-30B-A3B-3bit
run "30B-A3B-3bit SPEC (draft 0.6B, k=4)" --model mlx-community/Qwen3-30B-A3B-3bit --draft-model $DRAFT --num-draft-tokens 4

kill $RAMPID 2>/dev/null
echo "ALL_MLX_BENCH_DONE"
