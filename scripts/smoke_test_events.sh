#!/usr/bin/env bash
# Smoke test for WebSocket event bus (Section C)
# Usage: ./scripts/smoke_test_events.sh
#
# Prerequisites: websocat (brew install websocat)
# Starts permagentd on port 3001, connects to /events, verifies daemon_started.

set -euo pipefail

BINARY="$(dirname "$0")/../target/release/permagentd"
PORT=3001
HOST=127.0.0.1

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Release binary not found at $BINARY"
    echo "Run: cargo build --release -p permagent-daemon"
    exit 1
fi

if ! command -v websocat &> /dev/null; then
    echo "ERROR: websocat not found. Install with: brew install websocat"
    exit 1
fi

echo "==> Starting permagentd on $HOST:$PORT..."
GOOSE_SERVER__HOST=$HOST GOOSE_SERVER__PORT=$PORT \
    "$BINARY" agent &
DAEMON_PID=$!

# Give it time to bind
sleep 2

echo "==> Connecting to ws://$HOST:$PORT/events (5s timeout)..."
EVENTS=$(timeout 5 websocat -1 "ws://$HOST:$PORT/events" 2>/dev/null || true)

echo "==> First event received:"
echo "$EVENTS" | head -1

if echo "$EVENTS" | grep -q '"daemon_started"'; then
    echo "PASS: daemon_started event received"
else
    echo "FAIL: daemon_started not found in: $EVENTS"
fi

echo "==> Sending SIGTERM to daemon (PID $DAEMON_PID)..."
kill -TERM "$DAEMON_PID" 2>/dev/null || true
wait "$DAEMON_PID" 2>/dev/null || true

echo "==> Done."
