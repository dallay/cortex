"""Prepare the pinned MiniLM bundle inside a Docker volume.

The Muninn artifact contract verifies the bundle directory with native
descriptor checks. On Linux the check only accepts a local ext-family,
tmpfs or overlay filesystem, and it requires the bundle to be owned by the
process that opens it. A bind mount from a macOS host is a virtual
filesystem and fails the check, so the bundle is copied into a Docker
volume, which lives on a local filesystem inside the Linux virtual
machine.

The copy preserves the restrictive modes and the ownership of the
original, because the contract verifies both.
"""

from __future__ import annotations

from pathlib import Path

from .docker import DockerRunner
from .errors import BenchError

BUNDLE_VOLUME = "membench-minilm-bundle"
BUNDLE_ROOT = "/bundle"
_MARKER = ".membench-bundle-id"


def bundle_marker(bundle_id: str) -> str:
    """Return the marker content that identifies a prepared bundle."""

    return bundle_id


def prepare_bundle_volume(
    bundle_path: Path,
    bundle_id: str,
    image: str,
    uid: int,
    gid: int,
    timeout: float = 900.0,
) -> None:
    """Copy the pinned bundle into the Docker volume used by the topology.

    The copy is idempotent: a volume that already carries the same bundle
    identifier is left untouched.
    """

    if not bundle_path.is_dir():
        raise BenchError(f"embedding bundle not found: {bundle_path}")
    runner = DockerRunner.for_image(image)
    destination = f"{BUNDLE_ROOT}/{bundle_id}"
    script = (
        "set -eu; "
        # The copy is skipped only when the volume already carries this
        # bundle, its directory, and the ownership the contract verifies.
        f'if [ "$(cat {BUNDLE_ROOT}/{_MARKER} 2>/dev/null)" = "{bundle_id}" ] '
        f'&& [ -d "{destination}" ] && [ "$(stat -c %u {BUNDLE_ROOT})" = "{uid}" ]; '
        "then exit 0; fi; "
        f"rm -rf {BUNDLE_ROOT}/* {BUNDLE_ROOT}/.[!.]* 2>/dev/null || true; "
        f'mkdir -p "{destination}"; '
        f'cp -a /src/. "{destination}/"; '
        f"chown -R {uid}:{gid} {BUNDLE_ROOT}; "
        # The contract verifies the modes: private directories, read-only
        # files, and no permission for the group or for others.
        f'find "{destination}" -type d -exec chmod 0500 {{}} +; '
        f'find "{destination}" -type f -exec chmod 0400 {{}} +; '
        f'printf "%s" "{bundle_id}" > {BUNDLE_ROOT}/{_MARKER}; '
        f"chown {uid}:{gid} {BUNDLE_ROOT}/{_MARKER}; "
        f"chmod 0400 {BUNDLE_ROOT}/{_MARKER}; "
        f"chmod 0500 {BUNDLE_ROOT}"
    )
    result = runner.run(
        [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "--user",
            "0:0",
            "--volume",
            f"{BUNDLE_VOLUME}:/bundle",
            "--volume",
            f"{bundle_path}:/src:ro",
            "--entrypoint",
            "sh",
            image,
            "-c",
            script,
        ],
        timeout=timeout,
    )
    if not result.ok:
        raise BenchError(f"could not prepare the bundle volume: {result.summary()}")
