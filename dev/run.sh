#!/usr/bin/env bash
# =============================================================================
# run.sh — Quick dev container for rook
#
# Build a single rook image and run it locally to smoke-test the app.
# Nothing touches your host OS beyond the image itself and the ${DEFAULT_PORT} port.
#
# Usage:
#   dev/run.sh build         # Build the rook:dev image
#   dev/run.sh up            # Start the container (waits for /health)
#   dev/run.sh down          # Stop and remove the container
#   dev/run.sh logs          # Tail container logs
#   dev/run.sh shell         # Open a shell inside the container
#   dev/run.sh restart       # Restart the container
#   dev/run.sh status        # Show container + /health response
#   dev/run.sh clean         # Remove image and container
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

COMPOSE_FILE="dev/docker-compose.dev.yml"
IMAGE="rook:dev"
CONTAINER="rook-dev"
DEFAULT_PORT=8090
# Use 127.0.0.1 (not localhost) to avoid macOS IPv6-first resolution quirks
# where OrbStack only binds IPv4 for the host port.
HEALTH_URL="http://127.0.0.1:${DEFAULT_PORT}/health"
HEALTH_TIMEOUT=30

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info()  { echo -e "${GREEN}[INFO]${NC}  $*"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

# Wait for the /health endpoint to respond from the host
wait_healthy() {
    local attempt=1
    log_info "Waiting for /health on $HEALTH_URL ..."
    while [[ $attempt -le $HEALTH_TIMEOUT ]]; do
        if curl -sf "$HEALTH_URL" > /dev/null 2>&1; then
            log_info "rook is healthy!"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    log_error "rook did not become healthy in ${HEALTH_TIMEOUT}s."
    log_error "Check logs with: $0 logs"
    return 1
}

is_running() {
    docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${CONTAINER}$"
}

is_built() {
    docker image inspect "$IMAGE" > /dev/null 2>&1
}

# =============================================================================
# Test container commands
# =============================================================================

TEST_COMPOSE_FILE="dev/docker-compose.test.yml"
TEST_IMAGE_SERVER="rook:test-server"
TEST_IMAGE_CLIENT="rook:test-client"
TEST_CLIENT="rook-test-client"
TEST_SERVER_PORT=3773
TEST_HEALTH_URL="http://127.0.0.1:${TEST_SERVER_PORT}/health"
TEST_HEALTH_TIMEOUT=60

cmd_test_build() {
    log_info "Building test images..."
    docker compose -f "$TEST_COMPOSE_FILE" build
    log_info "Build complete: $TEST_IMAGE_SERVER + $TEST_IMAGE_CLIENT"
}

cmd_test_up() {
    if ! docker image inspect "$TEST_IMAGE_SERVER" > /dev/null 2>&1; then
        log_warn "Image $TEST_IMAGE_SERVER not found. Building it first..."
        cmd_test_build
    fi

    log_info "Starting test environment (rook-server + test client)..."
    docker compose -f "$TEST_COMPOSE_FILE" up -d

    log_info "Waiting for rook server to be healthy..."
    local attempt=1
    while [[ $attempt -le $TEST_HEALTH_TIMEOUT ]]; do
        if curl -sf "$TEST_HEALTH_URL" > /dev/null 2>&1; then
            log_info "rook server is healthy!"
            break
        fi
        sleep 2
        attempt=$((attempt + 1))
        echo -n "."
    done
    echo ""

    if [[ $attempt -gt $TEST_HEALTH_TIMEOUT ]]; then
        log_error "rook server did not become healthy in ${TEST_HEALTH_TIMEOUT}s."
        log_error "Check logs with: $0 test-logs"
        return 1
    fi

    log_info ""
    log_info "Test environment ready!"
    log_info "  Rook server: http://127.0.0.1:${TEST_SERVER_PORT}"
    log_info "  Dashboard:   http://127.0.0.1:${TEST_SERVER_PORT}/dashboard/"
    log_info ""
    log_info "Get API key:"
    echo "    docker exec rook-test-server cat /run/secrets/api_key"
    log_info ""
    log_info "Other commands:"
    echo "    $0 test-logs     # tail container logs"
    echo "    $0 test-shell    # shell into test client container"
    echo "    $0 test-down     # stop and remove"
}

cmd_test_down() {
    log_info "Stopping test containers..."
    docker compose -f "$TEST_COMPOSE_FILE" down
    log_info "Done."
}

cmd_test_logs() {
    docker compose -f "$TEST_COMPOSE_FILE" logs -f
}

cmd_test_shell() {
    if ! docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${TEST_CLIENT}$"; then
        log_error "$TEST_CLIENT is not running. Start it with: $0 test-up"
        exit 1
    fi
    docker exec -it "$TEST_CLIENT" /bin/bash
}

cmd_test_status() {
    log_info "Test environment status:"
    docker compose -f "$TEST_COMPOSE_FILE" ps

    echo ""
    log_info "Rook server /health:"
    if curl -sf "$TEST_HEALTH_URL" 2>/dev/null | python3 -m json.tool 2>/dev/null; then
        :
    else
        log_warn "(/health not responding yet)"
    fi

    echo ""
    log_info "Test client configuration:"
    docker exec "$TEST_CLIENT" cat /root/.config/opencode/opencode.json 2>/dev/null || log_warn "(test client not running)"
}

cmd_test_clean() {
    log_info "Removing test containers and images..."
    docker compose -f "$TEST_COMPOSE_FILE" down --rmi local --remove-orphans 2>/dev/null || true
    log_info "Clean complete."
}

# =============================================================================
# Commands
# =============================================================================

cmd_build() {
    log_info "Building $IMAGE image from dev/Dockerfile.dev..."
    docker build \
        --file "dev/Dockerfile.dev" \
        --tag "$IMAGE" \
        .
    log_info "Build complete: $IMAGE"
}

cmd_up() {
    if ! is_built; then
        log_warn "Image $IMAGE not found. Building it first..."
        cmd_build
    fi

    log_info "Starting $CONTAINER..."
    docker compose -f "$COMPOSE_FILE" up -d

    if wait_healthy; then
        log_info ""
        log_info "Try it:"
        echo "    curl $HEALTH_URL"
        log_info ""
        log_info "Other commands:"
        echo "    $0 logs      # tail container logs"
        echo "    $0 shell     # shell into the container"
        echo "    $0 down      # stop and remove"
    else
        log_error "Container started but /health did not respond."
        log_error "On macOS, prefer 127.0.0.1 over localhost (IPv6 quirk with OrbStack)."
        return 1
    fi
}

cmd_down() {
    log_info "Stopping $CONTAINER..."
    docker compose -f "$COMPOSE_FILE" down
    log_info "Done."
}

cmd_logs() {
    docker compose -f "$COMPOSE_FILE" logs -f
}

cmd_shell() {
    if ! is_running; then
        log_error "$CONTAINER is not running. Start it with: $0 up"
        exit 1
    fi
    docker exec -it "$CONTAINER" /bin/sh
}

cmd_restart() {
    log_info "Restarting $CONTAINER..."
    docker compose -f "$COMPOSE_FILE" restart
    wait_healthy
}

cmd_status() {
    if is_running; then
        log_info "$CONTAINER is RUNNING"
        docker ps --filter "name=^${CONTAINER}$" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
        echo ""
        log_info "/health response:"
        if curl -sf "$HEALTH_URL" | python3 -m json.tool 2>/dev/null; then
            :
        else
            log_warn "(/health not responding yet)"
        fi
    else
        log_warn "$CONTAINER is NOT running"
        if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER}$"; then
            log_info "  (container exists but stopped. Run: $0 up)"
        else
            log_info "  (no container. Run: $0 up)"
        fi
    fi
}

cmd_clean() {
    log_info "Removing container and image..."
    docker compose -f "$COMPOSE_FILE" down --rmi local --remove-orphans 2>/dev/null || true
    docker rmi "$IMAGE" 2>/dev/null || true
    log_info "Clean complete."
}

show_help() {
    cat << 'EOF'
dev/run.sh — Run rook in a local Docker container (no host pollution)

Dev commands (single rook server):
  dev/run.sh build         Build the rook:dev image
  dev/run.sh up            Start the container (waits until /health is OK)
  dev/run.sh down          Stop and remove the container
  dev/run.sh logs          Tail container logs
  dev/run.sh shell         Open a shell in the running container
  dev/run.sh restart       Restart the container
  dev/run.sh status        Show container + /health response
  dev/run.sh clean         Remove image and container

Test commands (rook server + opencode client):
  dev/run.sh test-build     Build test images (rook server + opencode client)
  dev/run.sh test-up        Start test environment and wait for healthy
  dev/run.sh test-down      Stop and remove test containers
  dev/run.sh test-logs      Tail test container logs
  dev/run.sh test-shell     Open a shell in the test client container
  dev/run.sh test-status    Show test environment status
  dev/run.sh test-clean     Remove test images and containers

The dev container uses dev/test-configs/rook-dev.toml and is fully ephemeral
(in-memory DB). It exposes port ${DEFAULT_PORT} on the host (127.0.0.1).

The test container uses dev/test-configs/rook-test.toml with a persistent
file-based DB and opencode pre-configured to use the rook server.

Examples:
  dev/run.sh up            # Start dev container and wait until healthy
  curl http://127.0.0.1:${DEFAULT_PORT}/health
  dev/run.sh test-up       # Start test environment with opencode
  dev/run.sh test-shell    # Bash into test client to run opencode
  dev/run.sh down          # Stop dev container
  dev/run.sh test-down     # Stop test containers
EOF
}

CMD="${1:-help}"
shift || true

case "$CMD" in
    build)       cmd_build ;;
    up)          cmd_up ;;
    down)        cmd_down ;;
    logs)        cmd_logs ;;
    shell)       cmd_shell ;;
    restart)     cmd_restart ;;
    status)      cmd_status ;;
    clean)       cmd_clean ;;
    test-build)  cmd_test_build ;;
    test-up)     cmd_test_up ;;
    test-down)   cmd_test_down ;;
    test-logs)   cmd_test_logs ;;
    test-shell)  cmd_test_shell ;;
    test-status) cmd_test_status ;;
    test-clean)  cmd_test_clean ;;
    help|--help|-h) show_help ;;
    *)       log_error "Unknown command: $CMD"; show_help; exit 1 ;;
esac
