# =============================================================================
# Runtime image - expects pre-built rook binary in the same directory
# =============================================================================
FROM debian:bookworm-slim

LABEL maintainer="Yuniel Acosta <yunielacosta738@gmail.com>"
LABEL org.opencontainers.image.source="https://github.com/dallay/cortex"
LABEL org.opencontainers.image.description="AI proxy/router for LLM requests with fallback, caching, and audit logging"

# Install runtime dependencies only
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    tzdata \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 rook

WORKDIR /app

# Copy pre-built binary (passed via docker-context)
COPY rook /usr/local/bin/rook

# Create config directory with proper permissions
RUN mkdir -p /app/config \
    && chown -R rook:rook /app

# Ensure binary is executable
RUN chmod +x /usr/local/bin/rook

USER rook

EXPOSE 8080

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/rook"]
CMD ["server", "--config", "/app/config/rook.toml"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://localhost:8080/health || exit 1