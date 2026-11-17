"""Fail-closed admission and byte snapshots for frozen C1 inputs.

This module does not execute inference or establish exporter provenance. A caller
must supply the trusted manifest digest obtained from an independently reviewed
record, then use snapshots as the bytes passed to each execution stage.
"""
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path, PurePosixPath
import re

import c1_freeze

EXPECTED_CONTRACT = "minilm-cpu-f32-v1"
HEX_SHA256 = re.compile(r"[0-9a-f]{64}")


class ProvenanceError(ValueError):
    """Frozen inputs, manifest identity, or byte provenance failed validation."""


@dataclass(frozen=True)
class AdmittedInputs:
    root: Path
    manifest_path: Path
    manifest_sha256: str
    contract: str
    entries: tuple[tuple[str, int, str], ...]
    input_paths: tuple[str, ...]


@dataclass(frozen=True)
class InputSnapshot:
    path: str
    data: bytes
    size: int
    sha256: str


def _pairs_no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ProvenanceError("manifest_duplicate_key")
        result[key] = value
    return result


def _sha256(data):
    return hashlib.sha256(data).hexdigest()


def admit(root, manifest_path, expected_manifest_sha256):
    """Authenticate manifest bytes and every listed file before execution."""
    if type(expected_manifest_sha256) is not str or not HEX_SHA256.fullmatch(expected_manifest_sha256):
        raise ProvenanceError("manifest_digest_argument")
    root = Path(root)
    manifest_path = Path(manifest_path)
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise ProvenanceError("workspace_root")
    if not manifest_path.is_absolute() or manifest_path.is_symlink() or not manifest_path.is_file():
        raise ProvenanceError("manifest_path")
    root = root.resolve(strict=True)
    manifest_path = manifest_path.resolve(strict=True)
    try:
        manifest_raw = manifest_path.read_bytes()
        if _sha256(manifest_raw) != expected_manifest_sha256:
            raise ProvenanceError("manifest_digest")
        manifest = json.loads(manifest_raw, object_pairs_hook=_pairs_no_duplicates)
        if (type(manifest) is not dict
                or type(manifest.get("schema_version")) is not int
                or manifest.get("schema_version") != c1_freeze.SCHEMA_VERSION
                or manifest.get("state") != c1_freeze.STATE
                or manifest.get("numeric_contract_version") != EXPECTED_CONTRACT
                or type(manifest.get("inputs")) is not list
                or not manifest["inputs"]):
            raise ProvenanceError("manifest_contract")
        entries = []
        seen = set()
        for entry in manifest["inputs"]:
            if type(entry) is not dict or set(entry) != {"path", "size", "sha256"}:
                raise ProvenanceError("manifest_entry")
            relative = entry["path"]
            if (type(relative) is not str or not relative or "\\" in relative
                    or PurePosixPath(relative).is_absolute()
                    or any(part in ("", ".", "..") for part in relative.split("/"))
                    or type(entry["size"]) is not int or entry["size"] < 0
                    or type(entry["sha256"]) is not str or not HEX_SHA256.fullmatch(entry["sha256"])):
                raise ProvenanceError("manifest_entry")
            if relative in seen:
                raise ProvenanceError("manifest_duplicate_path")
            seen.add(relative)
            path = root.joinpath(*PurePosixPath(relative).parts)
            if path.is_symlink() or not path.is_file():
                raise ProvenanceError("input_path")
            try:
                path.resolve(strict=True).relative_to(root)
            except (OSError, ValueError):
                raise ProvenanceError("input_outside_workspace") from None
            data = path.read_bytes()
            if len(data) != entry["size"] or _sha256(data) != entry["sha256"]:
                raise ProvenanceError("input_digest")
            entries.append((relative, entry["size"], entry["sha256"]))
    except ProvenanceError:
        raise
    except (OSError, ValueError, TypeError, RecursionError):
        raise ProvenanceError("manifest_invalid") from None
    return AdmittedInputs(root, manifest_path, expected_manifest_sha256,
                          EXPECTED_CONTRACT, tuple(entries), tuple(item[0] for item in entries))


def read_frozen_input(admitted, relative):
    """Read one listed file once and return its verified byte snapshot.

    The caller must pass ``snapshot.data`` to the execution stage. This function
    records verified bytes; it cannot prove what an external process consumed.
    """
    expected = next((entry for entry in admitted.entries if entry[0] == relative), None)
    if expected is None:
        raise ProvenanceError("input_not_admitted")
    if (type(relative) is not str or not relative or "\\" in relative
            or PurePosixPath(relative).is_absolute()
            or any(part in ("", ".", "..") for part in relative.split("/"))):
        raise ProvenanceError("input_path")
    path = admitted.root.joinpath(*PurePosixPath(relative).parts)
    try:
        current = admitted.root
        for part in PurePosixPath(relative).parts:
            current = current / part
            if current.is_symlink():
                raise ProvenanceError("input_path")
        if not path.is_file():
            raise ProvenanceError("input_path")
        path.resolve(strict=True).relative_to(admitted.root.resolve(strict=True))
        data = path.read_bytes()
    except OSError:
        raise ProvenanceError("input_read") from None
    if len(data) != expected[1] or _sha256(data) != expected[2]:
        raise ProvenanceError("input_digest")
    return InputSnapshot(relative, data, len(data), _sha256(data))


def verify_unchanged(admitted):
    """Recheck manifest and all frozen files after a run stage."""
    try:
        manifest_raw = admitted.manifest_path.read_bytes()
    except OSError:
        raise ProvenanceError("manifest_read") from None
    if _sha256(manifest_raw) != admitted.manifest_sha256:
        raise ProvenanceError("manifest_changed")
    try:
        manifest = json.loads(manifest_raw, object_pairs_hook=_pairs_no_duplicates)
        c1_freeze.verify(admitted.root, manifest)
    except (c1_freeze.FreezeError, ProvenanceError, OSError, ValueError, TypeError, RecursionError):
        raise ProvenanceError("frozen_inputs_changed") from None
