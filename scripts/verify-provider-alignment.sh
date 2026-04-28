#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

if [ -z "$ANTHROPIC_API_KEY" ]; then
  echo "FAIL: ANTHROPIC_API_KEY not set in environment"
  echo "  export ANTHROPIC_API_KEY=sk-ant-... and re-run"
  exit 1
fi

if [ ! -x "target/release/permagent" ]; then
  echo "FAIL: target/release/permagent not found"
  echo "  cargo build --release -p permagent-cli -p permagent-daemon"
  exit 1
fi

# Helper: wait for daemon /status endpoint to respond
wait_for_daemon() {
  local max_attempts=${1:-20}
  for i in $(seq 1 $max_attempts); do
    if curl -sf http://localhost:3001/status >/dev/null 2>&1; then
      echo "daemon ready after ${i}s"
      return 0
    fi
    sleep 1
  done
  echo "FAIL: daemon did not become ready after ${max_attempts}s"
  return 1
}

echo "=== Cleanup ==="
pkill -f permagentd 2>/dev/null || true
rm -rf ~/.permagent
launchctl unload ~/Library/LaunchAgents/ai.permagent.daemon.plist 2>/dev/null || true
rm -f ~/Library/LaunchAgents/ai.permagent.daemon.plist

echo "=== Test 1: Setup wizard writes config.yaml correctly ==="
target/release/permagent setup --non-interactive --provider anthropic --api-key "$ANTHROPIC_API_KEY" --agent-name test
wait_for_daemon 20 || exit 1

GOOSE_PROVIDER_VAL=$(grep "^GOOSE_PROVIDER:" ~/.permagent/config.yaml | awk '{print $2}')
GOOSE_MODEL_VAL=$(grep "^GOOSE_MODEL:" ~/.permagent/config.yaml | awk '{print $2}')
echo "config.yaml GOOSE_PROVIDER=$GOOSE_PROVIDER_VAL GOOSE_MODEL=$GOOSE_MODEL_VAL"
[ "$GOOSE_PROVIDER_VAL" = "anthropic" ] || { echo "FAIL: GOOSE_PROVIDER not set in config.yaml"; exit 1; }

echo "=== Test 2: Daemon /config returns same values ==="
DAEMON_PROVIDER=$(curl -s http://localhost:3001/config | python3 -c "import sys,json; print(json.load(sys.stdin)['config'].get('GOOSE_PROVIDER',''))")
echo "daemon /config GOOSE_PROVIDER=$DAEMON_PROVIDER"
[ "$DAEMON_PROVIDER" = "anthropic" ] || { echo "FAIL: daemon doesn't read GOOSE_PROVIDER"; exit 1; }

echo "=== Test 3: /config/providers shows anthropic as default ==="
DEFAULT_NAMES=$(curl -s http://localhost:3001/config/providers | python3 -c "import sys,json; data=json.load(sys.stdin); print(','.join(p['name'] for p in data if p.get('is_default')))")
echo "/config/providers default names=$DEFAULT_NAMES"
[ "$DEFAULT_NAMES" = "anthropic" ] || { echo "FAIL: anthropic not marked default"; exit 1; }

echo "=== Test 4: Mutation via /config/set_provider persists ==="
curl -s -X POST http://localhost:3001/config/set_provider \
  -H "Content-Type: application/json" \
  -d '{"provider":"anthropic","model":"claude-haiku-4-5"}'
sleep 1

NEW_MODEL=$(grep "^GOOSE_MODEL:" ~/.permagent/config.yaml | awk '{print $2}')
echo "after set_provider, GOOSE_MODEL=$NEW_MODEL"
[ "$NEW_MODEL" = "claude-haiku-4-5" ] || { echo "FAIL: set_provider didn't persist to config.yaml"; exit 1; }

echo "=== Test 5: launchctl reload preserves state ==="
launchctl unload ~/Library/LaunchAgents/ai.permagent.daemon.plist
sleep 2
launchctl load ~/Library/LaunchAgents/ai.permagent.daemon.plist
wait_for_daemon 20 || exit 1

DEFAULT_AFTER_RELOAD=$(curl -s http://localhost:3001/config/providers | python3 -c "import sys,json; data=json.load(sys.stdin); print(','.join(p['name'] for p in data if p.get('is_default')))")
echo "after launchctl reload, default=$DEFAULT_AFTER_RELOAD"
[ "$DEFAULT_AFTER_RELOAD" = "anthropic" ] || { echo "FAIL: launchctl reload didn't restore default provider"; exit 1; }

echo "=== Test 6: End-to-end chat works ==="
set +e

SID=$(curl -s -X POST http://localhost:3001/api/sessions -H "Content-Type: application/json" -d '{}' | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
RID=$(python3 -c "import uuid; print(uuid.uuid4())")
echo "Session: $SID, Request: $RID"

REPLY_RESPONSE=$(curl -s -X POST "http://localhost:3001/sessions/$SID/reply" \
  -H "Content-Type: application/json" \
  -d "{\"request_id\":\"$RID\",\"user_message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"say hi\"}],\"created\":$(date +%s),\"metadata\":{\"userVisible\":true,\"agentVisible\":true}}}")
echo "reply response: $REPLY_RESPONSE"

if ! echo "$REPLY_RESPONSE" | grep -q "request_id"; then
  echo "FAIL: reply endpoint didn't accept request"
  exit 1
fi

sleep 3
EVENTS_FILE=$(mktemp)
curl -N -s "http://localhost:3001/sessions/$SID/events" > "$EVENTS_FILE" 2>&1 &
CURL_PID=$!
sleep 15
kill $CURL_PID 2>/dev/null || true
wait $CURL_PID 2>/dev/null || true

EVENTS=$(cat "$EVENTS_FILE")
rm -f "$EVENTS_FILE"

MESSAGE_COUNT=$(echo "$EVENTS" | grep -c '"type":"Message"')
MESSAGE_COUNT=${MESSAGE_COUNT//[^0-9]/}
MESSAGE_COUNT=${MESSAGE_COUNT:-0}
echo "Test 6: received $MESSAGE_COUNT Message events"

if [ "$MESSAGE_COUNT" -lt 1 ]; then
  echo "FAIL: no Message events received from chat"
  echo "First 30 lines of SSE stream:"
  echo "$EVENTS" | head -30
  exit 1
fi

FIRST_TEXT=$(echo "$EVENTS" | grep '"type":"Message"' | head -1 | python3 -c "import sys,json,re; line=sys.stdin.read(); match=re.search(r'data: (.*)', line); data=json.loads(match.group(1)); print(data['message']['content'][0].get('text','(no text)'))" 2>/dev/null || echo "(parse failed)")
echo "First message content: $FIRST_TEXT"

set -e

echo ""
echo "=== ALL TESTS PASSED ==="
