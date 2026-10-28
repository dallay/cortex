#!/usr/bin/env bash
# Initialize the repository's CodeGraph index when the optional CLI is available.
set -euo pipefail

if command -v codegraph >/dev/null 2>&1; then
  codegraph init
else
  printf '%s\n' 'CodeGraph CLI not found; skipping index initialization. Install codegraph and rerun setup to enable it.'
fi
