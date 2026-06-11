#!/bin/bash
# =============================================================================
# E2E Test Runner for API Key CRUD
#
# Prerequisites:
#   - Docker must be running
#   - Build the image first: docker build -f Dockerfile.dev -t rook:e2e-api-keys .
#   - Dashboard deps installed: cd apps/rook/dashboard && pnpm install
#
# Usage:
#   ./dev/e2e/run-api-keys-e2e.sh           # Start container for manual testing
#   ./dev/e2e/run-api-keys-e2e.sh --test     # Run tests automatically
#   ./dev/e2e/run-api-keys-e2e.sh --cleanup # Clean up only
# =============================================================================

set -e

# Resolve repo root relative to script location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"

CONTAINER_NAME="rook-e2e-api-keys"
export API_PORT=3773
# Expose the host-side port to the dashboard (Vite proxy) and Playwright
# (global-setup). The rook container listens on 3773 internally and we map
# it to API_PORT on the host, so everything running OUTSIDE the container
# must use API_PORT, not 3773.
#
# Use 127.0.0.1 (not localhost) on purpose: on macOS, Docker's userland proxy
# only forwards IPv4, but `localhost` resolves to ::1 first, which causes
# ECONNRESET for every request. Forcing IPv4 via 127.0.0.1 sidesteps that.
export API_TARGET="http://127.0.0.1:${API_PORT}"
export API_BASE_URL="${API_TARGET}"
TEST_CONFIG="${REPO_ROOT}/dev/test-configs/rook-api-keys-test.toml"
ADMIN_PASSWORD="Admin123456-"
DASHBOARD_DIR="${REPO_ROOT}/apps/rook/dashboard"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

cleanup() {
    log_info "Cleaning up..."
    docker rm -f "$CONTAINER_NAME" 2>/dev/null || true
}

# Parse arguments
MODE="${1:-}"

if [ "$MODE" = "--cleanup" ]; then
    cleanup
    exit 0
fi

# Cleanup any existing container
cleanup

log_info "Building Docker image..."
cd "$REPO_ROOT"
docker build -f Dockerfile.dev -t rook:e2e-api-keys . 2>&1 | tail -5

log_info "Starting rook container..."
docker run -d \
    --name "$CONTAINER_NAME" \
    --tmpfs /tmp \
    -p ${API_PORT}:3773 \
    -v "$TEST_CONFIG:/app/rook.toml:ro" \
    -e ROOK_CONFIG=/app/rook.toml \
    -e RUST_LOG=info \
    -e API_KEY_HASH_SECRET="test-secret-for-e2e-testing-only" \
    -e RUST_BACKTRACE=1 \
    -e ENCRYPTION_PASSPHRASE="test-encryption-passphrase-for-e2e-tests-only" \
    -e ENCRYPTION_SALT="St7xmpfwUTbsXGBHIlYjvg" \
    rook:e2e-api-keys > /dev/null 2>&1

log_info "Waiting for server to be ready..."
for i in {1..30}; do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(1); s.connect(('localhost', $API_PORT)); s.close(); exit(0)" 2>/dev/null; then
        log_info "Server is ready on port $API_PORT"
        break
    fi
    if [ $i -eq 30 ]; then
        log_error "Server failed to start"
        docker logs "$CONTAINER_NAME" | tail -20
        cleanup
        exit 1
    fi
    sleep 1
done

log_info "Seeding admin password..."
if ! docker exec -u non-root "$CONTAINER_NAME" /usr/local/bin/rook seed-admin --config /app/rook.toml "$ADMIN_PASSWORD"; then
    log_error "Failed to seed admin password"
    docker logs "$CONTAINER_NAME" | tail -20
    cleanup
    exit 1
fi

log_info "Seeding test provider..."
# Login from host to get auth cookies, then create a test provider
# API is at 127.0.0.1:3773 (mapped from container's 3773)
# Use 127.0.0.1 explicitly — macOS Docker userland proxy only forwards IPv4
#
# Step 1: GET /login to get CSRF token and set the csrf_token cookie
CSRF_RESPONSE=$(curl -s -X GET "${API_TARGET}/login" -c /tmp/rook-e2e-cookies.txt)
CSRF_TOKEN=$(echo "$CSRF_RESPONSE" | jq -r '.csrf_token' 2>/dev/null || echo "")

if [ -z "$CSRF_TOKEN" ] || [ "$CSRF_TOKEN" = "null" ]; then
    log_error "Failed to get CSRF token from GET /login"
    cat /tmp/rook-e2e-cookies.txt 2>/dev/null || true
else
    log_info "Got CSRF token: ${CSRF_TOKEN:0:20}..."
fi

# Step 2: POST /login with CSRF token and cookie
LOGIN_RESPONSE=$(curl -s -X POST "${API_TARGET}/login" \
    -H "Content-Type: application/json" \
    -H "X-CSRF-Token: $CSRF_TOKEN" \
    -b /tmp/rook-e2e-cookies.txt \
    -c /tmp/rook-e2e-cookies.txt \
    -d "{\"username\":\"admin\",\"password\":\"$ADMIN_PASSWORD\"}")
AUTH_TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r '.csrf_token' 2>/dev/null || echo "")

if [ -z "$AUTH_TOKEN" ] || [ "$AUTH_TOKEN" = "null" ]; then
    log_warn "Login response didn't return expected format, checking cookies..."
    cat /tmp/rook-e2e-cookies.txt 2>/dev/null || true
else
    log_info "Login successful"
fi

# Step 3: Create provider with CSRF token
# IMPORTANT: after POST /login, server returns a NEW CSRF token in body (.csrf_token)
# The original token from GET /login was consumed during login, so use AUTH_TOKEN instead
AUTH_TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r '.csrf_token' 2>/dev/null || echo "")
if [ -z "$AUTH_TOKEN" ] || [ "$AUTH_TOKEN" = "null" ]; then
    log_warn "No new CSRF token from login, using original token"
    AUTH_TOKEN="$CSRF_TOKEN"
fi
log_info "Using CSRF token for provider creation: ${AUTH_TOKEN:0:20}..."

PROVIDER_RESPONSE=$(curl -s -X POST "${API_TARGET}/api/providers" \
    -H "Content-Type: application/json" \
    -H "X-CSRF-Token: $AUTH_TOKEN" \
    -b /tmp/rook-e2e-cookies.txt \
    -d '{
        "name": "test-openai",
        "providerKind": "openai",
        "providerRuntimeId": "00000000-0000-0000-0000-000000000001",
        "authType": "api_key",
        "credentials": {"apiKey": "test-key"},
        "isActive": true,
        "priority": 1,
        "config": {"maxConcurrent": 10, "quotaWindowThresholds": {"warning": 0.8, "error": 0.9}}
    }')
log_info "Provider creation response: $PROVIDER_RESPONSE"

# Clean up temp files
rm -f /tmp/rook-e2e-cookies.txt

log_info "Container ready!"
log_info "  API: http://localhost:$API_PORT"
log_info "  Admin: admin / $ADMIN_PASSWORD"
log_info ""

if [ "$MODE" = "--test" ]; then
    log_info "Running Playwright tests..."

    # Start dashboard dev server in background
    log_info "Starting dashboard dev server..."
    cd "$DASHBOARD_DIR"
    pnpm run dev > /tmp/dashboard-dev.log 2>&1 &
    DASHBOARD_PID=$!

    # Wait for dashboard to be ready
    for i in {1..30}; do
        if curl -sf http://localhost:4747 > /dev/null 2>&1; then
            log_info "Dashboard is ready on port 4747"
            break
        fi
        sleep 2
    done

    # Run Playwright tests
    cd "$DASHBOARD_DIR"
    if     ADMIN_PASSWORD="$ADMIN_PASSWORD" pnpm playwright test e2e/api-keys.spec.ts; then
        log_info "Tests passed!"
        TEST_RESULT=0
    else
        log_error "Tests failed!"
        TEST_RESULT=1
    fi

    # Cleanup
    log_info "Stopping dashboard..."
    kill $DASHBOARD_PID 2>/dev/null || true

    cleanup

    exit $TEST_RESULT
else
    echo "Press Ctrl+C to stop the container, or run with --test to run tests automatically"
    echo "To run tests manually:"
    echo "  1. cd $DASHBOARD_DIR"
    echo "  2. pnpm run dev"
    echo "  3. pnpm playwright test e2e/api-keys.spec.ts"
    echo ""

    # Wait for user interrupt - use docker logs to block
    trap cleanup INT TERM
    docker logs -f "$CONTAINER_NAME" &
    wait
fi
