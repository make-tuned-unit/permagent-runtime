#!/usr/bin/env bash
#
# reap-worktrees.sh — drain the worktree/branch graveyard (#581).
#
# WHY: worktree lanes and their branches accumulate after PRs merge — each is
# disk (a per-lane cargo target tree, #584), cognitive load, and a stale-build
# collision risk (#574). Detection without reaping doesn't drain the
# graveyard, so this tool does both — with the destructive half gated.
#
# WHAT IT DOES
#   1. MERGED-LANE REAP: for every worktree lane under --root whose branch's
#      PR has merged, remove the worktree, delete the local branch, and prune
#      the lane's .shared-target/<lane> tree. With --delete-remote, also
#      delete the remote branch.
#   2. IDLE SWEEP (report-only): branches with no commits in --age-days
#      (default 14) are surfaced for a human decision — never auto-deleted.
#
# SAFETY MODEL (explicit, not implied):
#   - Default mode is REPORT ONLY. Nothing is touched without --apply.
#   - A lane with uncommitted changes or unpushed commits is NEVER reaped,
#     even under --apply — that's someone's work.
#   - Merge detection asks `gh` for the branch's merged PR (squash-merges
#     leave no ancestry, so `merge-base --is-ancestor` alone under-detects);
#     without gh it falls back to the ancestor check and says so.
#   - Idle branches are reported, never deleted (the decision-surface rule:
#     destructive is proposed, not autonomous).
#
# Usage:
#   scripts/reap-worktrees.sh                      # report what would happen
#   scripts/reap-worktrees.sh --apply              # reap merged, clean lanes
#   scripts/reap-worktrees.sh --apply --delete-remote
#   scripts/reap-worktrees.sh --json               # machine-readable report
#                                                  # (Steward card input, #581)
#
# Exit codes: 0 = ok (report or reap complete), 2 = bad usage.
#
set -uo pipefail

MAIN_REPO="${MAIN_REPO:-$HOME/dev/permagent-runtime}"
WORKTREES_ROOT="${WORKTREES_ROOT:-$HOME/dev/permagent-worktrees}"
AGE_DAYS=14
APPLY=0
DELETE_REMOTE=0
JSON=0

log() { printf '[reap %s] %s\n' "$(date '+%H:%M:%S')" "$*" >&2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --root)          WORKTREES_ROOT="$2"; shift 2 ;;
    --repo)          MAIN_REPO="$2"; shift 2 ;;
    --age-days)      AGE_DAYS="$2"; shift 2 ;;
    --apply)         APPLY=1; shift ;;
    --delete-remote) DELETE_REMOTE=1; shift ;;
    --json)          JSON=1; shift ;;
    *) log "unknown arg: $1"; exit 2 ;;
  esac
done

have_gh=0
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  have_gh=1
else
  log "gh unavailable/unauthenticated — squash-merged branches will read as UNMERGED (ancestor check only)"
fi

# Is this branch merged? gh knows about squash merges; ancestry is the fallback.
branch_merged() {
  local branch="$1"
  if [ "$have_gh" -eq 1 ]; then
    local n
    n=$(gh pr list --repo "$(git -C "$MAIN_REPO" remote get-url origin)" \
        --head "$branch" --state merged --json number --jq 'length' 2>/dev/null || echo "")
    [ -n "$n" ] && [ "$n" -gt 0 ] && return 0
  fi
  git -C "$MAIN_REPO" merge-base --is-ancestor "$branch" origin/main 2>/dev/null
}

json_rows=""
emit() { # status lane branch detail
  if [ "$JSON" -eq 1 ]; then
    json_rows="${json_rows}${json_rows:+,}{\"status\":\"$1\",\"lane\":\"$2\",\"branch\":\"$3\",\"detail\":\"$4\"}"
  else
    printf '%-10s %-28s %-28s %s\n' "$1" "$2" "$3" "$4"
  fi
}

git -C "$MAIN_REPO" fetch --prune origin >/dev/null 2>&1 || log "WARNING: fetch failed; working from possibly-stale refs"

# ---- 1. worktree lanes -------------------------------------------------------
[ "$JSON" -eq 1 ] || printf '%-10s %-28s %-28s %s\n' STATUS LANE BRANCH DETAIL
current_path="" current_branch=""
while IFS= read -r line; do
  case "$line" in
    worktree\ *) current_path="${line#worktree }" ;;
    branch\ *)   current_branch="${line#branch refs/heads/}" ;;
    "")
      # end of one worktree record
      if [ -n "$current_path" ] && [ "$current_path" != "$MAIN_REPO" ]; then
        lane=$(basename "$current_path")
        b="${current_branch:-<detached>}"
        if [ -n "$(git -C "$current_path" status --porcelain 2>/dev/null)" ]; then
          emit KEEP "$lane" "$b" "uncommitted changes — never reaped"
        elif [ "$b" != "<detached>" ] && [ -n "$(git -C "$current_path" log --oneline "origin/$b..HEAD" 2>/dev/null | head -1)" ]; then
          emit KEEP "$lane" "$b" "unpushed commits — never reaped"
        elif [ "$b" != "<detached>" ] && branch_merged "$b"; then
          if [ "$APPLY" -eq 1 ]; then
            git -C "$MAIN_REPO" worktree remove "$current_path" \
              && git -C "$MAIN_REPO" branch -D "$b" >/dev/null \
              && rm -rf "$WORKTREES_ROOT/.shared-target/$lane"
            if [ "$DELETE_REMOTE" -eq 1 ]; then
              git -C "$MAIN_REPO" push -q origin --delete "$b" 2>/dev/null || true
            fi
            emit REAPED "$lane" "$b" "merged — worktree, branch, target tree removed"
          else
            emit REAPABLE "$lane" "$b" "merged + clean (run with --apply to reap)"
          fi
        else
          emit UNMERGED "$lane" "$b" "not merged — left alone"
        fi
      fi
      current_path="" current_branch=""
      ;;
  esac
done < <(git -C "$MAIN_REPO" worktree list --porcelain; echo)

# ---- 2. idle-branch sweep (report only) -------------------------------------
cutoff=$(( $(date +%s) - AGE_DAYS * 86400 ))
while IFS= read -r b; do
  [ "$b" = "main" ] && continue
  last=$(git -C "$MAIN_REPO" log -1 --format=%ct "$b" 2>/dev/null || echo 0)
  if [ "$last" -gt 0 ] && [ "$last" -lt "$cutoff" ]; then
    emit IDLE "-" "$b" "no commits in >${AGE_DAYS}d — surfaced for a human decision"
  fi
done < <(git -C "$MAIN_REPO" for-each-ref --format='%(refname:short)' refs/heads/)

if [ "$JSON" -eq 1 ]; then
  printf '{"age_days":%s,"apply":%s,"lanes":[%s]}\n' "$AGE_DAYS" "$APPLY" "$json_rows"
fi
