#!/bin/sh
# Runs the frozen Python C1 exporter with the local CPU reference virtualenv.
#
# The runner rejects a symlinked executable, and the virtualenv interpreter is
# a symlink into the Homebrew runtime. This wrapper is a regular file that
# resolves the virtualenv relative to its own location, so the attempt does not
# depend on an absolute path recorded outside the workspace.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec "$here/../.cache/minilm-cpu-oracle-venv/bin/python" "$@"
