#!/bin/bash
set -e

echo "=== Cleanup ==="
pkill -f permagentd 2>/dev/null || true
rm -rf ~/.permagent
launchctl unload ~/Library/LaunchAgents/ai.permagent.daemon.plist 2>/dev/null || true
rm -f ~/Library/LaunchAgents/ai.permagent.daemon.plist

echo "=== Test 1: Setup wizard writes config.yaml correctly ==="
target/release/permagent setup --non-interactive --provider anthropic --api-key "$ANTHROPIC_API_KEY" --agent-name test
sleep 5

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
sleep 5

DEFAULT_AFTER_RELOAD=$(curl -s http://localhost:3001/config/providers | python3 -c "import sys,json; data=json.load(sys.stdin); print(','.join(p['name'] for p in data if p.get('is_default')))")
echo "after launchctl reload, default=$DEFAULT_AFTER_RELOAD"
[ "$DEFAULT_AFTER_RELOAD" = "anthropic" ] || { echo "FAIL: launchctl reload didn't restore default provider"; exit 1; }

echo "=== Test 6: End-to-end chat works ==="
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

sleep 2
EVENTS=$(timeout 12 curl -N "http://localhost:3001/sessions/$SID/events" 2>&1)
MESSAGE_COUNT=$(echo "$EVENTS" | grep -c '"type":"Message"')
echo "Test 6: received $MESSAGE_COUNT Message events"
[ "$MESSAGE_COUNT" -gt 0 ] || { echo "FAIL: no Message events received from chat — daemon accepted request but Anthropic call did not produce streaming events"; exit 1; }

# Show first event content as proof
FIRST_TEXT=$(echo "$EVENTS" | grep '"type":"Message"' | head -1 | python3 -c "import sys,json; data=json.loads(sys.stdin.read().split('data: ')[1]); print(data['message']['content'][0].get('text','(no text)'))" 2>/dev/null)
echo "First message content: $FIRST_TEXT"

echo ""
echo "=== ALL TESTS PASSED ==="
