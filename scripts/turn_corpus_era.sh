#!/usr/bin/env bash
#
# turn_corpus_era.sh — census the `turn_events` corpus and record its era boundary.
#
# WHY: Spectral's pin bump adds `voided_at TEXT DEFAULT NULL` to `turn_events`.
# The moment it lands, every pre-existing row gets `voided_at = NULL` — byte-
# identical to a post-bump turn that completed normally and simply was not
# voided. The corpus can no longer describe its own eras, and a later "what
# fraction of turns were abandoned?" query silently scores pre-bump aborts as
# `unreported`, re-creating exactly the aborted-vs-ignored conflation the void
# verb exists to remove (Spectral dispatch 2026-08-06v).
#
# The fix costs one timestamp and is unrecoverable afterwards: record the
# boundary — the `delivered_at` of the first turn served by a bumped daemon.
# Rows below it mean "voiding was impossible here", not "was not voided".
#
# Usage:
#   scripts/turn_corpus_era.sh                 # census + bump status
#   scripts/turn_corpus_era.sh --snapshot FILE # also write the full enumeration
#
# Run it BEFORE installing a bumped daemon (captures the pre-void era) and
# AGAIN after the first post-bump turn (fixes the boundary). Both snapshots
# belong in docs/architecture/TURN_OUTCOME_PROTOTYPE.md.

set -euo pipefail

DB="${PERMAGENT_BRAIN_DB:-$HOME/.permagent/brain/memory.db}"
SNAPSHOT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --snapshot)
      SNAPSHOT="${2:?--snapshot needs a path}"
      shift 2
      ;;
    -h | --help)
      sed -n '3,25p' "$0"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

[[ -r "$DB" ]] || {
  echo "no readable brain at $DB" >&2
  exit 1
}

q() { sqlite3 "$DB" "$1"; }

# `voided_at` present ⇒ the bump landed AND the daemon opened the brain with it.
# An install date cannot prove the second half; this is the indicator Spectral
# and this repo agreed to share (dispatch 06u).
if q "SELECT 1 FROM pragma_table_info('turn_events') WHERE name = 'voided_at';" | grep -q 1; then
  BUMPED=yes
else
  BUMPED=no
fi

TOTAL=$(q "SELECT COUNT(*) FROM turn_events;")
COMMITTED=$(q "SELECT COUNT(*) FROM turn_events WHERE committed_at IS NOT NULL;")
FIRST=$(q "SELECT COALESCE(MIN(delivered_at), '-') FROM turn_events;")
LAST=$(q "SELECT COALESCE(MAX(delivered_at), '-') FROM turn_events;")

echo "brain:          $DB"
echo "voided_at:      $BUMPED (yes ⇒ bump landed and the daemon opened this brain with it)"
echo "rows:           $TOTAL total, $COMMITTED committed, $((TOTAL - COMMITTED)) uncommitted"
echo "delivered_at:   $FIRST .. $LAST"

if [[ "$BUMPED" == yes ]]; then
  VOIDED=$(q "SELECT COUNT(*) FROM turn_events WHERE voided_at IS NOT NULL;")
  echo "voided:         $VOIDED"
  echo
  echo "BOUNDARY: the first row delivered by the bumped daemon is the era boundary."
  echo "Compare \$LAST above against the pre-bump snapshot recorded in"
  echo "docs/architecture/TURN_OUTCOME_PROTOTYPE.md — rows delivered at or before"
  echo "that snapshot's last timestamp are pre-void, and their NULL voided_at means"
  echo "\"voiding was impossible\", not \"not voided\"."
else
  echo
  echo "PRE-VOID ERA. Every row above predates the void verb. Record this census in"
  echo "docs/architecture/TURN_OUTCOME_PROTOTYPE.md before installing a bumped daemon —"
  echo "after the bump these rows are indistinguishable from unvoided post-bump turns."
fi

if [[ -n "$SNAPSHOT" ]]; then
  {
    echo "# turn_events census — voided_at present: $BUMPED"
    echo "# rows: $TOTAL total, $COMMITTED committed"
    echo "# delivered_at: $FIRST .. $LAST"
    echo "# occurrence_id|delivered_at|committed_at|policy"
    q "SELECT occurrence_id || '|' || delivered_at || '|' || COALESCE(committed_at, '') || '|' || policy FROM turn_events ORDER BY delivered_at;"
  } >"$SNAPSHOT"
  echo
  echo "snapshot written: $SNAPSHOT"
fi
