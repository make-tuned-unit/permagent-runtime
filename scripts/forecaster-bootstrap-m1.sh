#!/usr/bin/env bash
#
# forecaster-bootstrap-m1.sh — install TimesFM on the M1 mini, idempotently.
#
# WHY A SECOND MACHINE: the design amendment of 2026-08-24 moved the model off
# the hub so it is not carrying it alongside the local LLMs. The model
# is 1.93 GB resident for ~20 s a week; small, but with no business competing
# with the machine the user is typing on, which also holds the local LLMs.
#
# WHY uv: the spike found that `uv venv --python /opt/homebrew/bin/python3.12`
# FAILS on these machines — Homebrew's CPython returns an empty
# platform.mac_ver() and uv refuses it outright. Ask uv for a MANAGED
# interpreter instead. That is the whole reason this script does not just run
# `python3 -m venv`.
#
# SCHEDULING: the M1 runs a nightly llama.cpp RPC split for the Librarian under
# launchd, 01:50-06:10, using ~8 GB of Metal. Do not run this, or the weekly
# forecast sweep, inside that window.
#
# Re-running is a no-op. Every step checks before it acts.
#
# Usage:
#   scripts/forecaster-bootstrap-m1.sh                       # default target
#   FORECASTER_SSH_TARGET=user@host scripts/forecaster-bootstrap-m1.sh
#   FORECASTER_SSH_TARGET=... scripts/forecaster-bootstrap-m1.sh --check
#
set -euo pipefail

# The Tailscale address, which is stable across reboots. The direct Ethernet
# link-local address in ~/.ssh/config is NOT — it changes every reboot, so it is
# a convenience for a human at a terminal and never a scheduled dependency.
# No default: an address is one person's network, not a product fact.
TARGET="${FORECASTER_SSH_TARGET:?set FORECASTER_SSH_TARGET to a user@host reachable over SSH}"
REMOTE_DIR="${FORECASTER_REMOTE_DIR:-.permagent/forecaster}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

# Free space needed there: 882 MB of weights + ~600 MB of venv, plus room.
MIN_FREE_GB="${MIN_FREE_GB:-4}"

say() { printf '[forecaster-bootstrap] %s\n' "$*" >&2; }

ssh_run() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new \
      "$TARGET" "$@"
}

say "target: $TARGET"
if ! ssh_run 'echo ok' >/dev/null 2>&1; then
  say "REFUSED: cannot reach $TARGET over ssh."
  say "  The Tailscale address is the one to use for anything scheduled; the"
  say "  link-local 'm1' alias in ~/.ssh/config changes on every reboot."
  exit 3
fi

free_gb="$(ssh_run "df -g /System/Volumes/Data | tail -1 | awk '{print \$4}'")"
say "free space there: ${free_gb} GiB"
if [ "${free_gb:-0}" -lt "$MIN_FREE_GB" ]; then
  say "REFUSED: needs at least ${MIN_FREE_GB} GiB free, found ${free_gb} GiB."
  exit 10
fi

if ssh_run "pgrep -f 'rpc-server|llama-server' >/dev/null 2>&1"; then
  say "REFUSED: the Librarian's llama.cpp split is running there right now."
  say "  It holds ~8 GB of Metal; wait until after 06:10 local."
  exit 11
fi

status() {
  ssh_run "
    test -x \$HOME/$REMOTE_DIR/venv/bin/python && echo VENV
    test -f \$HOME/$REMOTE_DIR/forecast.py && echo SCRIPT
    test -d \$HOME/.cache/huggingface/hub && echo WEIGHTS
    \$HOME/.local/bin/uv --version >/dev/null 2>&1 && echo UV
  " 2>/dev/null || true
}

before="$(status)"
if [ "$CHECK_ONLY" -eq 1 ]; then
  say "venv:    $(grep -q VENV <<<"$before" && echo present || echo absent)"
  say "script:  $(grep -q SCRIPT <<<"$before" && echo present || echo absent)"
  say "weights: $(grep -q WEIGHTS <<<"$before" && echo present || echo absent)"
  say "uv:      $(grep -q UV <<<"$before" && echo present || echo absent)"
  exit 0
fi

if ! grep -q UV <<<"$before"; then
  say "installing uv (managed CPython is required; Homebrew's is refused by uv)"
  ssh_run 'curl -LsSf https://astral.sh/uv/install.sh | sh' >/dev/null
fi

say "creating $REMOTE_DIR and its venv (idempotent)"
ssh_run "
  set -e
  mkdir -p \$HOME/$REMOTE_DIR
  export PATH=\$HOME/.local/bin:\$PATH
  if [ ! -x \$HOME/$REMOTE_DIR/venv/bin/python ]; then
    uv venv --python 3.12 \$HOME/$REMOTE_DIR/venv
  fi
  uv pip install --quiet --python \$HOME/$REMOTE_DIR/venv/bin/python 'timesfm[torch]'
"

say "copying forecast.py"
scp -q -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
    "$ROOT/scripts/forecaster/forecast.py" "$TARGET:$REMOTE_DIR/forecast.py"

say "warming the checkpoint (first run downloads 882 MB, several minutes)"
# The batch is built by python on the remote, not by shell string-mashing:
# BSD `seq -s,` appends a trailing separator, which produced invalid JSON and a
# warm-up that failed while the install itself had succeeded.
ssh_run "
  \$HOME/$REMOTE_DIR/venv/bin/python -c 'import json,sys; json.dump([{\"series_id\":\"warmup\",\"values\":[float(i%7) for i in range(128)],\"horizon\":4}], sys.stdout)' \
  | \$HOME/$REMOTE_DIR/venv/bin/python \$HOME/$REMOTE_DIR/forecast.py > /dev/null
"

after="$(status)"
say "venv:    $(grep -q VENV <<<"$after" && echo present || echo ABSENT)"
say "script:  $(grep -q SCRIPT <<<"$after" && echo present || echo ABSENT)"
say "weights: $(grep -q WEIGHTS <<<"$after" && echo present || echo ABSENT)"
say "done. Re-running this script changes nothing."
