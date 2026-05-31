# =============================================================================
# dev/Dockerfile — Build rook locally, run in minimal Docker container
#
# Prerequisites:
#   1. Build locally: cargo build --bin rook
#   2. Build image:   docker build -f dev/Dockerfile -t rook:e2e-ubuntu .
#   3. Run:          docker run -p 8081:8080 rook:e2e-ubuntu
#
# Multi-distro: copy target/debug/rook first, then build images for each distro:
#   docker build -f dev/Dockerfile.ubuntu   -t rook:e2e-ubuntu .
#   docker build -f dev/Dockerfile.alpine   -t rook:e2e-alpine .
# =============================================================================

FROM ubuntu:24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy pre-built binary from local target/
COPY target/debug/rook /usr/local/bin/rook
COPY dev/test-configs/rook-minimal.toml /app/rook.toml

RUN chmod +x /usr/local/bin/rook

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/rook"]
CMD ["--config", "/app/rook.toml"]
