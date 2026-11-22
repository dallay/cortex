#!/usr/bin/env bash
# Bazel target that runs the dashboard type check through the existing pnpm
# workflow. Dependency resolution stays with pnpm: this script never installs,
# updates or locks packages.
set -euo pipefail

source "${TEST_SRCDIR}/${TEST_WORKSPACE:-_main}/apps/rook/dashboard/bazel/lib.sh"

dashboard_prepare_workspace
dashboard_prepare_toolchain

exec pnpm --filter @dallay/rook-dashboard exec vue-tsc --noEmit
