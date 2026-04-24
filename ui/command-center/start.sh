#!/bin/bash
# Start Command Center UI and API

set -e
set -u

echo "Starting Command Center..."

API_HOST="${COMMAND_CENTER_API_HOST:-127.0.0.1}"
API_PORT="${COMMAND_CENTER_API_PORT:-9843}"
API_URL="http://${API_HOST}:${API_PORT}"

# Load env files (repo + OpenClaw runtime) so integrations are configured.
ENV_CANDIDATES=(
  ".env"
  ".env.local"
  "$HOME/.openclaw/workspace/henry-command-center/vault/config/.env"
  "$HOME/.openclaw/workspace/command-center-ui/data/.env"
)
for env_file in "${ENV_CANDIDATES[@]}"; do
  if [[ -f "$env_file" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "$env_file"
    set +a
  fi
done

# Optional override for runtime dir
if [[ -n "${COMMAND_CENTER_RUNTIME_DIR:-}" ]]; then
  mkdir -p "${COMMAND_CENTER_RUNTIME_DIR}"
fi

# Start API server in background
echo "Starting API server on ${API_URL}"
python3 api/server.py &
API_PID=$!

# Start Gmail OAuth callback listener (port 9844)
echo "Starting Gmail callback server on http://127.0.0.1:9844"
python3 api/gmail_callback_server.py &
GMAIL_CB_PID=$!

# Wait for API to start
sleep 2

# Check if API is running
if curl -fsS "${API_URL}/api/health" > /dev/null; then
    echo "✓ API server running"
else
    echo "✗ API server failed to start"
    kill "${API_PID:-}" 2>/dev/null || true
    kill "${GMAIL_CB_PID:-}" 2>/dev/null || true
    exit 1
fi

# Check Gmail callback listener
if curl -fsS "http://127.0.0.1:9844/health" > /dev/null; then
    echo "✓ Gmail callback server running"
else
    echo "✗ Gmail callback server failed to start"
    kill "${API_PID:-}" 2>/dev/null || true
    kill "${GMAIL_CB_PID:-}" 2>/dev/null || true
    exit 1
fi

# Start dev server
echo "Starting dev server..."
npm run dev &
DEV_PID=$!

# Cleanup on exit
cleanup() {
    echo "Shutting down..."
    kill "${API_PID:-}" 2>/dev/null || true
    kill "${GMAIL_CB_PID:-}" 2>/dev/null || true
    kill "${DEV_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Wait for user input
echo ""
echo "Command Center running!"
echo "  API: ${API_URL}"
echo "  UI:  http://localhost:5173"
echo ""
echo "Press Ctrl+C to stop"

wait
