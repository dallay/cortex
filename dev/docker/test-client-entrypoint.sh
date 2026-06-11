#!/bin/bash
# =============================================================================
# dev/docker/test-client-entrypoint.sh — Test client startup
#
# Copies opencode.json from /tmp, injects the API key from /run/secrets/api_key,
# then execs the original CMD.
# =============================================================================

set -e

API_KEY_FILE="/run/secrets/api_key"
CONFIG_SOURCE="/tmp/opencode-test.json"
CONFIG_TARGET="/root/.config/opencode/opencode.json"

# If there's an original config and an API key file, inject the key
if [ -f "$CONFIG_SOURCE" ] && [ -f "$API_KEY_FILE" ]; then
    API_KEY=$(cat "$API_KEY_FILE")
    mkdir -p "$(dirname "$CONFIG_TARGET")"
    jq --arg api_key "$API_KEY" \
        '.provider.rook.options.apiKey = $api_key' \
        "$CONFIG_SOURCE" > "$CONFIG_TARGET"
    echo "[entrypoint] Updated opencode.json with API key from secrets"
fi

exec "$@"