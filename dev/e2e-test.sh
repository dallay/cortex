#!/usr/bin/env bash
# =============================================================================
# e2e-test.sh — Run rook e2e tests inside Docker containers
#
# Usage:
#   dev/e2e-test.sh build          # Build all Docker images
#   dev/e2e-test.sh up             # Start all containers
#   dev/e2e-test.sh down           # Stop all containers
#   dev/e2e-test.sh test [distro]  # Run e2e tests (all or specific distro)
#   dev/e2e-test.sh health [distro]# Check /health endpoint
#   dev/e2e-test.sh shell [distro]  # Shell into a container
#   dev/e2e-test.sh clean          # Remove images and containers
#
# Distros: ubuntu, alpine
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

DISTROS=(ubuntu alpine)
DEFAULT_PORT_UBUNTU=8081
DEFAULT_PORT_ALPINE=8082

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info()  { echo -e "${GREEN}[INFO]${NC}  $*"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

# Get port for distro
port_for() {
    case "$1" in
        ubuntu)  echo "$DEFAULT_PORT_UBUNTU" ;;
        alpine)  echo "$DEFAULT_PORT_ALPINE" ;;
        *)       echo "8080" ;;
    esac
}

# Wait for container to be healthy
wait_healthy() {
    local distro=$1
    local port; port=$(port_for "$distro")
    local max_attempts=20
    local attempt=1

    log_info "Waiting for rook-$distro to be healthy on port $port..."
    while [[ $attempt -le $max_attempts ]]; do
        if curl -sf "http://localhost:$port/health" > /dev/null 2>&1; then
            log_info "rook-$distro is healthy!"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done

    log_error "rook-$distro failed to become healthy after ${max_attempts}s"
    return 1
}

# =============================================================================
# Commands
# =============================================================================

cmd_build() {
    log_info "Building Docker images..."
    for distro in "${DISTROS[@]}"; do
        log_info "Building rook:e2e-$distro ..."
        docker build \
            --file "dev/Dockerfile.$distro" \
            --tag "rook:e2e-$distro" \
            .
    done
    log_info "Build complete!"
}

cmd_up() {
    log_info "Starting containers..."
    docker compose -f dev/docker-compose.yml up -d
    for distro in "${DISTROS[@]}"; do
        wait_healthy "$distro" || true
    done
    log_info "Containers started. Rook endpoints:"
    for distro in "${DISTROS[@]}"; do
        local port; port=$(port_for "$distro")
        echo "  - rook-$distro: http://localhost:$port"
    done
}

cmd_down() {
    log_info "Stopping containers..."
    docker compose -f dev/docker-compose.yml down
    log_info "Done."
}

cmd_test() {
    local distro="${1:-all}"

    # Ensure containers are up
    if ! docker compose -f dev/docker-compose.yml ps --status running | grep -q "rook-e2e"; then
        log_warn "Containers not running. Starting them first..."
        cmd_up
    fi

    if [[ "$distro" == "all" ]]; then
        for d in "${DISTROS[@]}"; do
            cmd_test "$d"
        done
        return
    fi

    local port; port=$(port_for "$distro")
    local base="http://localhost:$port"
    local failed=0

    log_info "=========================================="
    log_info "E2E tests for rook-$distro ($base)"
    log_info "=========================================="

    # Test 6.4: /health response shape
    log_info "--- Test 6.4: /health response shape ---"
    local health_response; health_response=$(curl -s "$base/health")
    echo "$health_response" | python3 -m json.tool > /dev/null 2>&1 && {
        log_info "  PASS: /health returns valid JSON"
        local has_status=$(echo "$health_response" | python3 -c "import sys,json; d=json.load(sys.stdin); print('ok' if 'status' in d and 'providers' in d else 'fail')" 2>/dev/null || echo "fail")
        if [[ "$has_status" == "ok" ]]; then
            log_info "  PASS: /health has 'status' and 'providers' fields"
        else
            log_error "  FAIL: /health missing required fields"
            failed=$((failed + 1))
        fi
    } || {
        log_error "  FAIL: /health did not return valid JSON"
        failed=$((failed + 1))
    }

    # Test 6.5: empty registry handled gracefully
    log_info "--- Test 6.5: empty registry graceful handling ---"
    local empty_status; empty_status=$(curl -s "$base/health" | python3 -c "import sys,json; print(json.load(sys.stdin)['status'])" 2>/dev/null || echo "error")
    if [[ "$empty_status" == "no_providers_configured" ]]; then
        log_info "  PASS: empty registry returns 'no_providers_configured'"
    else
        log_error "  FAIL: expected 'no_providers_configured', got '$empty_status'"
        failed=$((failed + 1))
    fi

    # Summary
    echo ""
    if [[ $failed -eq 0 ]]; then
        log_info "All tests PASSED for $distro"
    else
        log_error "$failed test(s) FAILED for $distro"
    fi
}

cmd_health() {
    local distro="${1:-ubuntu}"
    local port; port=$(port_for "$distro")
    local url="http://localhost:$port/health"

    log_info "Checking /health on rook-$distro ($url)..."
    curl -s "$url" | python3 -m json.tool
}

cmd_shell() {
    local distro="${1:-ubuntu}"
    log_info "Opening shell in rook-$distro container..."
    docker exec -it "rook-e2e-$distro" /bin/sh
}

cmd_clean() {
    log_info "Removing Docker images and containers..."
    docker compose -f dev/docker-compose.yml down --rmi all --remove-orphans 2>/dev/null || true
    docker rmi "rook:e2e-ubuntu" "rook:e2e-alpine" 2>/dev/null || true
    log_info "Clean complete."
}

# =============================================================================
# Main
# =============================================================================

show_help() {
    cat << 'EOF'
dev/e2e-test.sh — Run rook e2e tests inside Docker

Commands:
  dev/e2e-test.sh build          Build all Docker images
  dev/e2e-test.sh up             Start all containers
  dev/e2e-test.sh down          Stop all containers
  dev/e2e-test.sh test [distro]  Run e2e tests (all or ubuntu/alpine)
  dev/e2e-test.sh health [distro] Check /health endpoint
  dev/e2e-test.sh shell [distro]  Shell into a container
  dev/e2e-test.sh clean          Remove images and containers

Examples:
  dev/e2e-test.sh build           # Build images (one-time)
  dev/e2e-test.sh up              # Start containers
  dev/e2e-test.sh test            # Run tests on all distros
  dev/e2e-test.sh test ubuntu    # Run tests on ubuntu only
  dev/e2e-test.sh health alpine   # Check alpine /health
  dev/e2e-test.sh down            # Stop containers
  dev/e2e-test.sh clean          # Remove everything
EOF
}

CMD="${1:-help}"
shift || true

case "$CMD" in
    build)  cmd_build ;;
    up)     cmd_up ;;
    down)   cmd_down ;;
    test)   cmd_test "${1:-all}" ;;
    health) cmd_health "${1:-ubuntu}" ;;
    shell)  cmd_shell "${1:-ubuntu}" ;;
    clean)  cmd_clean ;;
    help|--help|-h) show_help ;;
    *)      log_error "Unknown command: $CMD"; show_help; exit 1 ;;
esac
