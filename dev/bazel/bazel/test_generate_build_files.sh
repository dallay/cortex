#!/usr/bin/env bash
# Bazel target that runs the unit tests of the BUILD file generator through the
# standard library test runner. The generator stays the source of truth for the
# Bazel build graph; this target only executes its tests.
set -euo pipefail

source "${TEST_SRCDIR}/${TEST_WORKSPACE:-_main}/dev/bazel/bazel/lib.sh"

bazel_prepare_workspace
bazel_prepare_python

exec python3 -m unittest dev/bazel/test_generate_build_files.py "$@"
