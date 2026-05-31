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

CONTAINER_NAME="rook-e2e-api-keys"
API_PORT=8081
TEST_CONFIG="/Users/acosta/Dev/dallay/cortex/dev/test-configs/rook-api-keys-test.toml"
ADMIN_PASSWORD="admin123"
DASHBOARD_DIR="/Users/acosta/Dev/dallay/cortex/apps/rook/dashboard"

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
cd /Users/acosta/Dev/dallay/cortex
docker build -f Dockerfile.dev -t rook:e2e-api-keys . 2>&1 | tail -5

log_info "Starting rook container..."
docker run -d \
    --name "$CONTAINER_NAME" \
    -p ${API_PORT}:8080 \
    -v "$TEST_CONFIG:/app/rook.toml:ro" \
    -e RUST_LOG=info \
    -e API_KEY_HASH_SECRET="test-secret-for-e2e-testing-only" \
    -e RUST_BACKTRACE=1 \
    --user root \
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
docker exec "$CONTAINER_NAME" /usr/local/bin/rook seed-admin "$ADMIN_PASSWORD" > /dev/null 2>&1

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
        if curl -sf http://localhost:5173 > /dev/null 2>&1; then
            log_info "Dashboard is ready on port 5173"
            break
        fi
        sleep 2
    done
    
    # Run Playwright tests
    cd "$DASHBOARD_DIR"
    if pnpm playwright test e2e/api-keys.spec.ts; then
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
    
    # Wait for user interrupt
    trap cleanup INT TERM
    wait
fi
