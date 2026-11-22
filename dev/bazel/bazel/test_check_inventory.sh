#!/usr/bin/env bash
# Bazel target that runs the unit tests of the Cargo to Bazel inventory
# checker through the standard library test runner. The checker stays the source
# of truth for the inventory mapping; this target only executes its tests.
set -euo pipefail

source "${TEST_SRCDIR}/${TEST_WORKSPACE:-_main}/dev/bazel/bazel/lib.sh"

bazel_prepare_workspace
bazel_prepare_python

exec python3 -m unittest dev/bazel/test_check_inventory.py "$@"
