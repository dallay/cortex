# =============================================================================
# Runtime image - expects pre-built rook binary in the same directory
# Uses TARGETARCH to select the correct binary (amd64 or arm64)
# =============================================================================
FROM debian:bookworm-slim

LABEL maintainer="Yuniel Acosta <info@yunielacosta.com>"
LABEL org.opencontainers.image.source="https://github.com/dallay/cortex"
LABEL org.opencontainers.image.description="AI proxy/router for LLM requests with fallback, caching, and audit logging"

# Install runtime dependencies only
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN useradd -m -u 1000 rook

WORKDIR /app

# Copy pre-built binary based on target architecture
# docker-context/amd64/rook-amd64 or docker-context/arm64/rook-arm64
ARG TARGETARCH
RUN echo "Target architecture: ${TARGETARCH}"
COPY rook-${TARGETARCH} /usr/local/bin/rook

# Create config directory with proper permissions and ensure binary is executable
RUN mkdir -p /app/config \
    && chown -R rook:rook /app \
    && chmod +x /usr/local/bin/rook

USER rook

EXPOSE 8080

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/rook"]
CMD ["server", "--config", "/app/config/rook.toml"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://localhost:8080/health || exit 1
