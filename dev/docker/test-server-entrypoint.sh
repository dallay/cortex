#!/bin/bash
# =============================================================================
# dev/docker/test-server-entrypoint.sh — Bootstrap test server
#
# 1. Start server in background with log file
# 2. Extract setup token from log file
# 3. Get CSRF token via GET /login
# 4. Call bootstrap endpoint to create API key
# 5. Kill background server and exec fresh server with seeded DB
# =============================================================================

set -e

SERVER_URL="${SERVER_URL:-http://localhost:3773}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-TestAdmin@123}"
MAX_RETRIES=30
RETRY_INTERVAL=1
LOG_FILE="/tmp/rook.log"
COOKIE_JAR="/tmp/cookies.txt"

echo "[entrypoint] Starting rook server in background..."
/usr/local/bin/rook > "$LOG_FILE" 2>&1 &
SERVER_PID=$!

cleanup() {
    if kill -0 $SERVER_PID 2>/dev/null; then
        kill $SERVER_PID 2>/dev/null || true
    fi
}
trap cleanup EXIT

echo "[entrypoint] Waiting for setup token..."
SETUP_TOKEN=""
for i in $(seq 1 $MAX_RETRIES); do
    # Extract setup token from the log file
    SETUP_TOKEN=$(grep -oP "rk-setup-[a-f0-9]+" "$LOG_FILE" 2>/dev/null | head -1 || true)
    if [ -n "$SETUP_TOKEN" ]; then
        echo "[entrypoint] Found setup token: ${SETUP_TOKEN:0:20}..."
        break
    fi
    if [ $i -eq $MAX_RETRIES ]; then
        echo "[entrypoint] Failed to get setup token in time"
        cat "$LOG_FILE" 2>/dev/null || true
        exit 1
    fi
    sleep $RETRY_INTERVAL
done

# Wait a bit more for server to be fully ready
sleep 2

echo "[entrypoint] Getting CSRF token..."
LOGIN_RESPONSE=$(curl -s -c "$COOKIE_JAR" -b "$COOKIE_JAR" "$SERVER_URL/login")
CSRF_TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r '.csrf_token // empty')

if [ -z "$CSRF_TOKEN" ]; then
    echo "[entrypoint] Failed to get CSRF token: $LOGIN_RESPONSE"
    exit 1
fi
echo "[entrypoint] Got CSRF token: ${CSRF_TOKEN:0:15}..."

echo "[entrypoint] Calling bootstrap endpoint to create API key..."
RESPONSE=$(curl -s -X POST "$SERVER_URL/api/bootstrap/setup" \
    -H "Content-Type: application/json" \
    -H "X-CSRF-Token: $CSRF_TOKEN" \
    -b "$COOKIE_JAR" \
    -c "$COOKIE_JAR" \
    -d "{\"setup_token\": \"$SETUP_TOKEN\", \"password\": \"$ADMIN_PASSWORD\"}")

API_KEY=$(echo "$RESPONSE" | jq -r '.api_key // empty')

if [ -z "$API_KEY" ]; then
    echo "[entrypoint] Failed to get API key: $RESPONSE"
    exit 1
fi

echo "[entrypoint] API key created: ${API_KEY:0:15}..."
echo "$API_KEY" > /run/secrets/api_key
chmod 600 /run/secrets/api_key

echo "[entrypoint] Stopping background server..."
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

echo "[entrypoint] Starting rook server..."
exec /usr/local/bin/rook