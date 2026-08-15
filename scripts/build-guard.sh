#!/usr/bin/env bash
#
# build-guard.sh — refuse-below-threshold free-space check + concurrent-build
# lock for cargo builds (#560, item 5).
#
# WHY: on the 16 GB dev machine this repo builds on, disk exhaustion bit THREE
# times in one day — including a cross-session collision where two agents built
# into the same shared CARGO_TARGET_DIR and clobbered each other mid-write. Two
# class-fixes, both here:
#   (a) FREE-SPACE GUARD — check the Data volume before building and refuse (or
#       warn) below a threshold, so a build fails fast and loud instead of
#       ENOSPC-wedging halfway through and corrupting the target dir.
#   (b) BUILD LOCK — serialize concurrent builds that share a target dir, keyed
#       on the RESOLVED target dir path. Two builds into the SAME target dir
#       serialize (no clobber); two builds into DIFFERENT dirs (per-worktree
#       CARGO_TARGET_DIR) do not block each other. This makes both isolation
#       strategies safe with one mechanism.
#
# Portable lock: macOS has no `flock(1)`, so this uses an atomic `mkdir` lock
# with PID liveness (a dead holder's lock is stolen) — no external deps.
#
#   (c) LANE ISOLATION (#584) — a worktree lane building into a target dir that
#       isn't its own can produce FALSE-GREEN test results: cargo deems another
#       lane's binary fresh, skips recompiling, and runs the other branch's
#       tests. When invoked from ~/dev/permagent-worktrees/<lane>, this guard
#       REFUSES to build unless the resolved target dir is namespaced by the
#       lane (the standing rule: CARGO_TARGET_DIR=…/.shared-target/<lane>).
#       Override with --allow-shared-target / ALLOW_SHARED_TARGET=1 — but a
#       green from a shared tree is not evidence.
#
# Usage:
#   scripts/build-guard.sh -- cargo check -p permagent-daemon
#   scripts/build-guard.sh --min-free-gb 15 -- cargo build --release
#   scripts/build-guard.sh --warn-only -- cargo check          # warn, don't refuse
#   scripts/build-guard.sh --allow-shared-target -- cargo check  # skip lane check
#   MIN_FREE_GB=20 scripts/build-guard.sh -- cargo test
#
# Exit codes: 10 = refused (low disk), 11 = lock acquire timeout, 12 = refused
# (worktree lane on a non-lane target dir), else the wrapped command's own
# exit code.
#
set -uo pipefail

MIN_FREE_GB="${MIN_FREE_GB:-10}"      # refuse below this many GiB free
WARN_ONLY=0
LOCK_TIMEOUT="${LOCK_TIMEOUT:-1800}"  # max seconds to wait for the build lock
DATA_VOL="${DATA_VOL:-/System/Volumes/Data}"
ALLOW_SHARED_TARGET="${ALLOW_SHARED_TARGET:-0}"
# Derived from where this checkout actually lives, not a guessed absolute path.
# It was hardcoded to ~/dev/permagent-worktrees until 2026-08-14, while the
# worktrees sat under ~/Documents/dev/permagent-worktrees — so the lane check
# below matched nothing and #584 silently protected nothing on this machine. A
# guard that believes it is guarding is worse than no guard.
WORKTREES_ROOT="${WORKTREES_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/permagent-worktrees}"

log() { printf '[build-guard %s] %s\n' "$(date '+%H:%M:%S')" "$*" >&2; }

# ---- args ------------------------------------------------------------------
while [ $# -gt 0 ]; do
  case "$1" in
    --min-free-gb)         MIN_FREE_GB="$2"; shift 2 ;;
    --warn-only)           WARN_ONLY=1; shift ;;
    --allow-shared-target) ALLOW_SHARED_TARGET=1; shift ;;
    --)                    shift; break ;;
    *)                     break ;;
  esac
done
if [ $# -eq 0 ]; then
  log "no command given. Usage: build-guard.sh [--min-free-gb N] [--warn-only] -- <cmd...>"
  exit 2
fi

# ---- (a) free-space guard --------------------------------------------------
avail_kb=$(df -k "$DATA_VOL" 2>/dev/null | awk 'NR==2{print $4}')
if [ -z "${avail_kb:-}" ]; then
  log "WARNING: could not read free space on $DATA_VOL — proceeding without the disk check"
else
  avail_gb=$(( avail_kb / 1024 / 1024 ))
  if [ "$avail_gb" -lt "$MIN_FREE_GB" ]; then
    if [ "$WARN_ONLY" -eq 1 ]; then
      log "WARNING: only ${avail_gb} GiB free on $DATA_VOL (threshold ${MIN_FREE_GB} GiB) — proceeding (--warn-only)"
    else
      log "REFUSING BUILD: only ${avail_gb} GiB free on $DATA_VOL, below the ${MIN_FREE_GB} GiB threshold."
      log "Free space (clear a target dir / caches) or re-run with --warn-only to override."
      exit 10
    fi
  else
    log "disk ok: ${avail_gb} GiB free on $DATA_VOL (threshold ${MIN_FREE_GB} GiB)"
  fi
fi

# ---- (b) build lock, keyed on the resolved target dir ----------------------
# Resolve the effective target dir the way cargo ACTUALLY would. `cargo
# metadata` honors CARGO_TARGET_DIR *and* any walked-up .cargo/config.toml
# `[build] target-dir` — the env-only guess this used to make missed the
# shared-config case entirely: two lanes without the env var keyed to
# different locks while cargo built both into ONE shared tree, so the lock
# never engaged for exactly the collision it was built for (#584).
target_dir=""
if command -v cargo >/dev/null 2>&1; then
  target_dir=$(cargo metadata --no-deps --format-version 1 2>/dev/null \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
fi
if [ -z "$target_dir" ]; then
  # Not a cargo project (or cargo unavailable): fall back to the env-var guess.
  target_dir="${CARGO_TARGET_DIR:-$PWD/target}"
fi

# ---- (c) lane isolation (#584): a worktree lane must own its target dir ----
case "$PWD" in
  "$WORKTREES_ROOT"/*)
    lane="${PWD#"$WORKTREES_ROOT"/}"
    lane="${lane%%/*}"
    case "$target_dir" in
      *"/$lane"|*"/$lane/"*)
        log "lane isolation ok: worktree '$lane' builds into $target_dir"
        ;;
      *)
        if [ "$ALLOW_SHARED_TARGET" = "1" ]; then
          log "WARNING: worktree '$lane' is building into a NON-LANE target dir ($target_dir) — a green from a shared tree can be another branch's tests (#584). Proceeding (--allow-shared-target)."
        else
          log "REFUSING BUILD: worktree lane '$lane' would build into $target_dir, which is not namespaced by the lane."
          log "A shared target tree can run ANOTHER worktree's test binary and report false-green (#584)."
          log "Fix: export CARGO_TARGET_DIR=\"$WORKTREES_ROOT/.shared-target/$lane\"  (the standing per-lane rule)"
          log "Override (not evidence-grade): --allow-shared-target or ALLOW_SHARED_TARGET=1."
          exit 12
        fi
        ;;
    esac
    ;;
esac
# A filesystem-safe key for this target dir (shasum -> hex; fall back to a
# sanitized path if shasum is unavailable).
if command -v shasum >/dev/null 2>&1; then
  key=$(printf '%s' "$target_dir" | shasum | awk '{print $1}')
else
  key=$(printf '%s' "$target_dir" | tr -c 'A-Za-z0-9' '_')
fi
LOCK_DIR="${TMPDIR:-/tmp}/permagent-build-guard.${key}.lock"

acquire_lock() {
  local waited=0
  while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    # Someone holds it. Steal it if the holder PID is dead (stale lock).
    local holder
    holder=$(cat "$LOCK_DIR/pid" 2>/dev/null || echo "")
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      log "stealing stale build lock from dead PID $holder"
      rm -rf "$LOCK_DIR"
      continue
    fi
    if [ "$waited" -ge "$LOCK_TIMEOUT" ]; then
      log "TIMEOUT waiting ${LOCK_TIMEOUT}s for build lock on $target_dir (held by PID ${holder:-?})"
      exit 11
    fi
    if [ "$waited" -eq 0 ]; then
      log "another build is using $target_dir (PID ${holder:-?}); waiting up to ${LOCK_TIMEOUT}s…"
    fi
    sleep 5
    waited=$(( waited + 5 ))
  done
  printf '%s\n' "$$" > "$LOCK_DIR/pid"
  log "acquired build lock for $target_dir"
}

# shellcheck disable=SC2329  # invoked indirectly via the EXIT/INT/TERM trap
release_lock() {
  # Only remove our own lock.
  if [ "$(cat "$LOCK_DIR/pid" 2>/dev/null || echo)" = "$$" ]; then
    rm -rf "$LOCK_DIR"
  fi
}

acquire_lock
trap release_lock EXIT INT TERM

# ---- run the wrapped command ----------------------------------------------
log "running: $*"
"$@"
status=$?
log "command exited with status $status"
exit "$status"
