#!/usr/bin/env bash
# Bazel target that builds the dashboard using Vite through the existing pnpm
# toolchain. Dependency resolution stays with pnpm: this script never installs,
# updates or locks packages.
set -euo pipefail

# When run via `bazel run`, the script executes from Bazel's output tree.
# Bazel's execroot contains symlinks to the real source tree. We locate the
# execroot by walking up from the script's directory until we find MODULE.bazel,
# then resolve the dashboard source directory through those symlinks.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

execroot_dir=""
check_dir="$SCRIPT_DIR"
while [[ "$check_dir" != "/" ]]; do
  if [[ -f "$check_dir/MODULE.bazel" ]]; then
    execroot_dir="$check_dir"
    break
  fi
  check_dir="$(dirname "$check_dir")"
done

if [[ -z "$execroot_dir" ]]; then
  echo "Could not locate Bazel execroot from script path: $SCRIPT_DIR" >&2
  exit 1
fi

# The execroot's apps/rook/dashboard may be a symlink to the real source tree.
# `pwd -P` resolves the physical path through all symlinks.
DASHBOARD_DIR="$(cd "$execroot_dir/apps/rook/dashboard" && pwd -P)"

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
