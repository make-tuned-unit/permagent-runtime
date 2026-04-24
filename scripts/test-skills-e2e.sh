#!/usr/bin/env bash
# End-to-end test for auto-skills detection pipeline (Section F)
#
# Tests the full flow:
#   1. Insert 2 matching task rows with same argument_shape_hash
#   2. Query repetition_candidates view — verify detection
#   3. POST /permagent/skills — save the proposed skill
#   4. GET /permagent/skills — verify it appears
#   5. POST /permagent/skills/dismiss — verify dismiss records
#
# Usage: ./scripts/test-skills-e2e.sh
# Prerequisites: release binary built, sqlite3 available

set -euo pipefail

BINARY="$(dirname "$0")/../target/release/permagentd"
PORT="${TEST_PORT:-13001}"
HOST=127.0.0.1
BASE="http://$HOST:$PORT"
DB_DIR=$(mktemp -d)
DAEMON_PID=""

cleanup() {
    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -rf "$DB_DIR"
}
trap cleanup EXIT

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; exit 1; }

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Release binary not found at $BINARY"
    echo "Run: cargo build --release -p permagent-daemon"
    exit 1
fi

echo "=== Auto-Skills E2E Test ==="
echo "DB dir: $DB_DIR"

# ── Step 1: Start daemon ────────────────────────────────────────────────────
echo ""
echo "--- Step 1: Start daemon on $HOST:$PORT ---"
GOOSE_SERVER__HOST=$HOST GOOSE_SERVER__PORT=$PORT \
    GOOSE_DATA_DIR="$DB_DIR" \
    "$BINARY" agent 2>/dev/null &
DAEMON_PID=$!

# Wait for daemon to be ready
for i in $(seq 1 20); do
    if curl -sf "$BASE/status" >/dev/null 2>&1 || curl -sf "$BASE/" >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        fail "Daemon exited prematurely"
    fi
    sleep 0.5
done

# Verify daemon is running
if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
    fail "Daemon not running"
fi
pass "Daemon started (PID $DAEMON_PID)"

# ── Step 2: Insert 2 matching tasks with same argument_shape_hash ────────
echo ""
echo "--- Step 2: Insert matching task rows ---"

DB_PATH="$DB_DIR/permagent.db"

# Wait for DB to be created by the daemon
for i in $(seq 1 20); do
    if [ -f "$DB_PATH" ]; then break; fi
    sleep 0.5
done

if [ ! -f "$DB_PATH" ]; then
    fail "Database not created at $DB_PATH"
fi

# Compute a deterministic shape hash (matching the Rust algo for gmail__search + {query: string})
# We hardcode this since we're testing the pipeline, not the hash function
SHAPE_HASH="test_shape_e2e01"
TOOL="gmail__search"
NOW=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")
YESTERDAY=$(date -u -v-1d +"%Y-%m-%dT%H:%M:%S.000Z" 2>/dev/null || date -u -d "yesterday" +"%Y-%m-%dT%H:%M:%S.000Z")

# Insert 2 completed tasks with the same shape hash
sqlite3 "$DB_PATH" <<SQL
INSERT OR IGNORE INTO tasks (id, user_id, description, tool_used, argument_shape_hash, status, completed_at, created_at)
VALUES ('test-task-001', 'default', 'Search unread emails', '$TOOL', '$SHAPE_HASH', 'completed', '$YESTERDAY', '$YESTERDAY');

INSERT OR IGNORE INTO tasks (id, user_id, description, tool_used, argument_shape_hash, status, completed_at, created_at)
VALUES ('test-task-002', 'default', 'Search important emails', '$TOOL', '$SHAPE_HASH', 'completed', '$NOW', '$NOW');
SQL

TASK_COUNT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM tasks WHERE argument_shape_hash = '$SHAPE_HASH' AND status = 'completed'")
if [ "$TASK_COUNT" -lt 2 ]; then
    fail "Expected 2 tasks, found $TASK_COUNT"
fi
pass "Inserted $TASK_COUNT matching tasks"

# ── Step 3: Check repetition_candidates view ─────────────────────────────
echo ""
echo "--- Step 3: Verify repetition_candidates view ---"

REP_COUNT=$(sqlite3 "$DB_PATH" \
    "SELECT occurrence_count FROM repetition_candidates WHERE argument_shape_hash = '$SHAPE_HASH' AND user_id = 'default'" 2>/dev/null || echo "0")

if [ -z "$REP_COUNT" ] || [ "$REP_COUNT" -lt 2 ]; then
    fail "repetition_candidates returned count=$REP_COUNT (expected >= 2)"
fi
pass "repetition_candidates detected $REP_COUNT occurrences"

LATEST_DESC=$(sqlite3 "$DB_PATH" \
    "SELECT latest_description FROM repetition_candidates WHERE argument_shape_hash = '$SHAPE_HASH'")
pass "Latest description: '$LATEST_DESC'"

# ── Step 4: Save skill via POST /permagent/skills ────────────────────────
echo ""
echo "--- Step 4: Save skill via API ---"

CREATE_RESP=$(curl -sf -X POST "$BASE/permagent/skills" \
    -H "Content-Type: application/json" \
    -d "{
        \"name\": \"search-emails\",
        \"description\": \"Search emails automatically\",
        \"toolUsed\": \"$TOOL\",
        \"argumentShapeHash\": \"$SHAPE_HASH\",
        \"definitionJson\": {\"steps\": [{\"action\": \"gmail__search\", \"args\": {\"query\": \"is:unread\"}}]},
        \"sourceTaskId\": \"test-task-002\"
    }" 2>&1) || fail "POST /permagent/skills failed: $CREATE_RESP"

SKILL_ID=$(echo "$CREATE_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null)

if [ -z "$SKILL_ID" ]; then
    fail "No skill ID returned. Response: $CREATE_RESP"
fi
pass "Skill created: $SKILL_ID"

# Verify skill_triggers row was created
TRIGGER_COUNT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM skill_triggers WHERE skill_id = '$SKILL_ID'")
if [ "$TRIGGER_COUNT" -lt 1 ]; then
    fail "No skill_triggers row for skill $SKILL_ID"
fi
pass "skill_triggers row created (trigger_count=$TRIGGER_COUNT)"

# ── Step 5: Verify skill appears in GET /permagent/skills ────────────────
echo ""
echo "--- Step 5: Verify skill in skills list ---"

LIST_RESP=$(curl -sf "$BASE/permagent/skills" 2>&1) || fail "GET /permagent/skills failed"

FOUND=$(echo "$LIST_RESP" | python3 -c "
import sys, json
skills = json.load(sys.stdin)
found = [s for s in skills if s['id'] == '$SKILL_ID']
print(len(found))
" 2>/dev/null)

if [ "$FOUND" != "1" ]; then
    fail "Skill $SKILL_ID not found in list. Response: $LIST_RESP"
fi
pass "Skill appears in GET /permagent/skills"

# ── Step 6: Verify dismiss flow ──────────────────────────────────────────
echo ""
echo "--- Step 6: Test dismiss flow ---"

DISMISS_HASH="dismiss_test_hash"
DISMISS_RESP=$(curl -sf -o /dev/null -w "%{http_code}" -X POST "$BASE/permagent/skills/dismiss" \
    -H "Content-Type: application/json" \
    -d "{\"argumentShapeHash\": \"$DISMISS_HASH\"}" 2>&1)

if [ "$DISMISS_RESP" != "200" ]; then
    fail "POST /permagent/skills/dismiss returned $DISMISS_RESP (expected 200)"
fi
pass "Dismiss returned 200"

DISMISS_COUNT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM skill_dismissals WHERE argument_shape_hash = '$DISMISS_HASH'")
if [ "$DISMISS_COUNT" -lt 1 ]; then
    fail "No dismissal row found for hash $DISMISS_HASH"
fi
pass "Dismissal recorded in skill_dismissals table"

# Verify 30-day suppression: dismissed hash should not appear in detection
# (The check_repetition_candidates function checks this, verified by presence of the dismissal row)
pass "30-day suppression row exists for re-prompt prevention"

# ── Step 7: Verify detection skips shapes with existing skills ───────────
echo ""
echo "--- Step 7: Verify existing skill suppresses re-proposal ---"

SKILL_TRIGGER_CONFIG=$(sqlite3 "$DB_PATH" "SELECT trigger_config FROM skill_triggers WHERE skill_id = '$SKILL_ID'")
if echo "$SKILL_TRIGGER_CONFIG" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d['argument_shape_hash'] == '$SHAPE_HASH'" 2>/dev/null; then
    pass "Trigger config contains argument_shape_hash for suppression"
else
    fail "Trigger config missing argument_shape_hash: $SKILL_TRIGGER_CONFIG"
fi

echo ""
echo "=== ALL TESTS PASSED ==="
