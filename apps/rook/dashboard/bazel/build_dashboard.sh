#!/usr/bin/env bash
# Bazel target that builds the dashboard using Vite through the existing pnpm
# toolchain. Dependency resolution stays with pnpm: this script never installs,
# updates or locks packages.
set -euo pipefail

# `bazel run` provides the source workspace root, independent of runfiles layout.
if [[ -z "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
  echo "BUILD_WORKSPACE_DIRECTORY must be set; run this target with bazel run." >&2
  exit 1
fi
DASHBOARD_DIR="$(cd "$BUILD_WORKSPACE_DIRECTORY/apps/rook/dashboard" && pwd -P)"

cd "$DASHBOARD_DIR"

# Prepare the Node/pnpm toolchain on PATH.
for candidate in \
  "${HOME}/Library/pnpm/bin" \
  "${HOME}/.local/share/pnpm" \
  "${HOME}/.local/pnpm" \
  "/opt/homebrew/bin" \
  "/usr/local/bin"; do
  if [[ -d "${candidate}" && ":${PATH}:" != *":${candidate}:"* ]]; then
    PATH="${PATH}:${candidate}"
  fi
done
export PATH

if [[ ! -d node_modules ]]; then
  echo "dashboard node_modules is missing; run 'pnpm install' (or 'just dashboard-install') first." >&2
  exit 1
fi

# Vite writes to dist/ (see vite.config.ts). The build is non-hermetic and
# modifies the source tree, which is acceptable for this non-sandboxed target.
pnpm --filter @dallay/rook-dashboard exec vite build
