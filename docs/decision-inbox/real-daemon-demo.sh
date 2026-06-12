#!/usr/bin/env bash
# Decision Inbox L4 — real-daemon end-to-end demo (coordinator integration evidence).
#
# Runs the freshly built daemon on a THROWAWAY data root, seeds decisions
# (incl. a goal card carrying an L2 verification digest), builds the UI with
# the real client pointed at the daemon, and captures Playwright evidence:
# home-card count, inbox list, approve end-to-end, and the 409 conflict path.
#
# Written because the L4 sandbox blocks long-running daemon processes —
# run this from a normal shell. NEVER touches the live data root.
#
# Usage:  bash docs/decision-inbox/real-daemon-demo.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
ROOT="$(mktemp -d /tmp/di-l4-demo.XXXXXX)"   # throwaway data root
PORT=3899
TARGET_DIR="$(cd "$REPO" && cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
DAEMON="$TARGET_DIR/debug/permagentd"
DB="$ROOT/spectral/permagent.db"
OUT="$REPO/docs/decision-inbox/screenshots-l4-real"

cleanup() {
  [[ -n "${DAEMON_PID:-}" ]] && kill "$DAEMON_PID" 2>/dev/null || true
  rm -rf "$ROOT"
  echo "throwaway root removed: $ROOT"
}
trap cleanup EXIT

# ── 1. Build + start the daemon on the throwaway root ──
(cd "$REPO" && cargo build -p permagent-daemon)
PERMAGENT_PATH_ROOT="$ROOT" \
DYLD_FALLBACK_LIBRARY_PATH=/Applications/Permagent.app/Contents/Frameworks \
RUST_LOG=info "$DAEMON" agent --port "$PORT" > "$ROOT/daemon.log" 2>&1 &
DAEMON_PID=$!

# Wait for boot + auto-generated bearer token.
for _ in $(seq 1 60); do
  [[ -f "$ROOT/secrets/daemon_token.json" ]] && curl -fsS "http://127.0.0.1:$PORT/status" >/dev/null 2>&1 && break
  sleep 1
done
TOKEN="$(python3 -c "import json;print(json.load(open('$ROOT/secrets/daemon_token.json'))['token'])")"
echo "daemon up (pid $DAEMON_PID), token loaded"

# ── 2. Seed: goal card with an L2 digest + open decisions ──
# The daemon's own startup created the schema and the personal project.
PROJECT_ID="$(sqlite3 "$DB" "SELECT id FROM projects LIMIT 1")"
COLUMN_ID="$(sqlite3 "$DB" "SELECT id FROM board_columns WHERE project_id='$PROJECT_ID' LIMIT 1")"

DIGEST='{"version":1,"goal_id":"g-demo","goal_title":"Decision queue saving","checks_summary":{"passed_count":4,"total_count":4,"tests_run":412,"one_line":"All 4 automated checks passed (412 tests)"},"verifier_summary":"Henry'\''s reviewer confirmed the work matches what was approved","costs":{"cost_usd":4.02,"accumulated_total_tokens":1900000,"worker_session_id":"s-demo","attempt_count":1},"checks":[{"type":"command_exit_zero","summary":"cargo test --workspace","status":"pass","output_excerpt":"exit: 0\n412 passed"}],"diff":{"files_changed":14,"insertions":482,"deletions":97,"per_file":[]},"out_of_path_files":[],"verifier":{"status":"pass","rationale":"Scope matches.","model":"qwen2.5:7b"},"started_at":"2026-06-12T00:00:00Z","finished_at":"2026-06-12T00:00:10Z"}'

sqlite3 "$DB" <<SQL
INSERT INTO cards (id, project_id, card_type, title, description, column_id, position, created_by, metadata_json)
VALUES ('g-demo', '$PROJECT_ID', 'goal', 'Decision queue saving', 'demo goal', '$COLUMN_ID', 0, 'system',
        json_object('verification', json('$DIGEST')));
INSERT INTO decisions (id, kind, goal_id, project_id, tier, headline, detail, payload_json, rank)
VALUES ('dec-demo-1', 'approve_review', 'g-demo', '$PROJECT_ID', 2,
        'Ship the change: decision queue saving',
        'Worker reported success; the goal moved to Review. Approve (Review → Complete) or reject (rework).',
        '{}', 0.9);
INSERT INTO decisions (id, kind, tier, headline, detail, payload_json, rank)
VALUES ('dec-demo-2', 'choice', 2,
        'Pick a path: where to keep your decision history',
        'Two workable options with different trade-offs.',
        '{"question":"Where should history live?","options":[{"id":"db","label":"Existing database"},{"id":"file","label":"Separate file"}],"default":"db"}', 0.8);
INSERT INTO decisions (id, kind, tier, headline, detail, payload_json, rank)
VALUES ('dec-demo-409', 'risk_gate', 2,
        'Spend a little more: experiment wants ten dollars extra',
        'Budget gate.', '{"action_class":"budget_increase","description":"raise budget","requested_by":"Henry"}', 0.7);
SQL
echo "seeded: 3 open decisions + goal card with verification digest"

# Sanity: real envelope from the real daemon.
curl -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/api/decisions" | python3 -m json.tool | head -30

# Pre-answer the 409 fixture out-of-band so the UI click hits the conflict path.
curl -fsS -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"answer":"reject"}' "http://127.0.0.1:$PORT/api/decisions/dec-demo-409/answer" >/dev/null
echo "dec-demo-409 pre-answered (next UI answer returns 409)"

# ── 3. Build the UI against the real daemon (real client is the default) ──
(cd "$REPO/ui/command-center" \
  && npm install --no-save playwright \
  && npx playwright install chromium \
  && VITE_DAEMON_URL="http://127.0.0.1:$PORT" npx vite build --outDir dist-real --emptyOutDir)

# ── 4. Playwright capture against the live daemon ──
mkdir -p "$OUT"
DEMO_PORT="$PORT" DEMO_TOKEN="$TOKEN" CAPTURE_OUT="$OUT" \
  node "$REPO/docs/decision-inbox/capture-l4-real.cjs"
rm -rf "$REPO/ui/command-center/dist-real"

echo "evidence → $OUT"
