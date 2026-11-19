#!/usr/bin/env python3
"""Check explicit Cargo package/target mappings to Bazel labels or reviewed exclusions.

This checker measures declaration inventory coverage only. It does not establish feature,
build, artifact, platform, or runtime test parity.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MAPPING = ROOT / "dev/bazel/cargo_inventory.json"

# Cargo reports path dependencies with the absolute location of the checkout.
# A committed inventory cannot carry that location, because the value would only
# match on the machine that wrote it, so the location is replaced by this marker.
WORKSPACE_PLACEHOLDER = "<workspace>"


class InventoryError(ValueError):
    pass


def portable_package_id(package_id: str, relative_directory: str) -> str:
    """Return a package ID that does not embed the absolute checkout location.

    Only the location changes between checkouts: the fragment after `#` is the
    package identity. The fragment is therefore preserved and only the path is
    replaced, which keeps the identity stable across machines and worktrees.
    """
    prefix = "path+file://"
    if not package_id.startswith(prefix):
        return package_id
    _, separator, fragment = package_id[len(prefix):].partition("#")
    if not separator:
        return package_id
    location = WORKSPACE_PLACEHOLDER
    if relative_directory not in ("", "."):
        location = f"{WORKSPACE_PLACEHOLDER}/{relative_directory}"
    return f"{prefix}{location}#{fragment}"


def _run(command: list[str], *, cwd: Path = ROOT) -> str:
    try:
        result = subprocess.run(command, cwd=cwd, check=True, capture_output=True, text=True)
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", "") or str(error)
        raise InventoryError(f"command failed: {' '.join(command)}: {detail.strip()}") from error
    return result.stdout


def _require_object(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise InventoryError(f"{context} must be an object")
    return value


def _require_string(value: Any, field: str, context: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise InventoryError(f"{context} {field} must be a non-empty string")
    return value


def cargo_inventory(metadata: dict[str, Any]) -> list[dict[str, str]]:
    metadata = _require_object(metadata, "Cargo metadata")
    packages_value = metadata.get("packages")
    members = metadata.get("workspace_members")
    if not isinstance(packages_value, list):
        raise InventoryError("metadata must contain packages array")
    if not isinstance(members, list) or not all(isinstance(item, str) for item in members):
        raise InventoryError("metadata must contain workspace_members string array")

    packages: dict[str, dict[str, Any]] = {}
    for index, value in enumerate(packages_value):
        package = _require_object(value, f"metadata package {index}")
        package_id = _require_string(package.get("id"), "id", f"metadata package {index}")
        if package_id in packages:
            raise InventoryError(f"duplicate Cargo package ID: {package_id}")
        packages[package_id] = package

    rows: list[dict[str, str]] = []
    for member_id in members:
        package = packages.get(member_id)
        if package is None:
            raise InventoryError(f"workspace member has no package record: {member_id}")
        manifest_raw = _require_string(package.get("manifest_path"), "manifest_path", f"package {member_id}")
        manifest = Path(manifest_raw)
        try:
            manifest_path = manifest.resolve().relative_to(ROOT.resolve()).as_posix()
        except ValueError as error:
            raise InventoryError(f"workspace manifest is outside repository: {manifest}") from error
        name = _require_string(package.get("name"), "name", f"package {member_id}")
        targets = package.get("targets")
        if not isinstance(targets, list):
            raise InventoryError(f"package {member_id} targets must be an array")
        for index, target_value in enumerate(targets):
            target = _require_object(target_value, f"package {member_id} target {index}")
            target_name = _require_string(target.get("name"), "name", f"package {member_id} target {index}")
            kinds = target.get("kind")
            if not isinstance(kinds, list) or not kinds or not all(isinstance(kind, str) and kind for kind in kinds):
                raise InventoryError(f"package {member_id} target {target_name} kind must be a non-empty string array")
            rows.append({
                "package": name,
                "package_id": portable_package_id(member_id, Path(manifest_path).parent.as_posix()),
                "package_version": f"{name}@{package.get('version', '')}",
                "manifest": manifest_path,
                "target": target_name,
                "kinds": ",".join(sorted(kinds)),
            })
    return sorted(rows, key=lambda row: (row["manifest"], row["package_id"], row["target"], row["kinds"]))


def identity(row: dict[str, str]) -> tuple[str, str, str, str]:
    return row["manifest"], row["package_id"], row["target"], row["kinds"]


def validate(metadata: dict[str, Any], mapping: dict[str, Any], labels: set[str]) -> list[dict[str, Any]]:
    inventory = cargo_inventory(metadata)
    by_identity: dict[tuple[str, str, str, str], dict[str, str]] = {}
    for row in inventory:
        key = identity(row)
        if key in by_identity:
            raise InventoryError(f"duplicate Cargo inventory identity: {key[0]}::{key[1]} [{key[2]}]")
        by_identity[key] = row
    mapping = _require_object(mapping, "mapping")
    entries = mapping.get("entries")
    if not isinstance(entries, list):
        raise InventoryError("mapping must contain an entries array")

    errors: list[str] = []
    seen: set[tuple[str, str, str, str]] = set()
    coverage: list[dict[str, Any]] = []
    for index, entry_value in enumerate(entries):
        if not isinstance(entry_value, dict):
            errors.append(f"mapping entry {index} must be an object")
            continue
        try:
            key = tuple(_require_string(entry_value.get(field), field, f"mapping entry {index}")
                        for field in ("manifest", "package_id", "target", "kinds"))
        except InventoryError as error:
            errors.append(str(error))
            continue
        if key in seen:
            errors.append(f"duplicate mapping: {key[0]}::{key[1]} [{key[2]}]")
            continue
        seen.add(key)
        row = by_identity.get(key)
        if row is None:
            errors.append(f"stale Cargo inventory identity: {key[0]}::{key[1]} [{key[2]}]")
            continue
        label = entry_value.get("label")
        exclusion = entry_value.get("exclusion")
        valid_label = isinstance(label, str) and bool(label.strip())
        valid_exclusion = isinstance(exclusion, str) and bool(exclusion.strip())
        if valid_label == valid_exclusion:
            errors.append(f"mapping requires exactly one label or reviewed exclusion: {key[0]}::{key[1]}")
            continue
        if valid_label:
            if label not in labels:
                errors.append(f"stale Bazel label: {label} for {key[0]}::{key[1]}")
                continue
            coverage.append({**row, "status": "mapped", "bazel_label": label})
        else:
            coverage.append({**row, "status": "excluded", "reason": exclusion.strip()})

    for row in inventory:
        if identity(row) not in seen:
            errors.append(f"missing mapping: {row['manifest']}::{row['target']} [{row['kinds']}]")

    if errors:
        raise InventoryError("\n".join(sorted(errors)))
    return sorted(coverage, key=lambda row: (row["manifest"], row["target"], row["kinds"]))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mapping", type=Path, default=DEFAULT_MAPPING)
    parser.add_argument("--metadata-fixture", type=Path, help="Use Cargo metadata JSON instead of running Cargo")
    parser.add_argument("--query-fixture", type=Path, help="Read Bazel query labels, one per line, instead of running Bazel")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        metadata = json.loads(args.metadata_fixture.read_text() if args.metadata_fixture else _run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))
        mapping = json.loads(args.mapping.read_text())
        label_output = args.query_fixture.read_text() if args.query_fixture else _run(
            ["bazel", "query", "//...", "--output=label", "--lockfile_mode=error"])
        labels = {line.strip() for line in label_output.splitlines() if line.strip() and not line.startswith("#")}
        coverage = validate(metadata, mapping, labels)
    except (OSError, json.JSONDecodeError, InventoryError) as error:
        print(f"inventory check failed: {error}", file=sys.stderr)
        return 1

    if args.format == "json":
        print(json.dumps({"scope": "Cargo package/target declaration inventory only", "coverage": coverage},
                         sort_keys=True, indent=2))
    else:
        for row in coverage:
            detail = row["bazel_label"] if row["status"] == "mapped" else f"excluded: {row['reason']}"
            print(f"{row['status']:8} {row['manifest']}::{row['target']} [{row['kinds']}] -> {detail}")
        mapped = sum(row["status"] == "mapped" for row in coverage)
        print(f"Inventory coverage: {mapped}/{len(coverage)} mapped; {len(coverage) - mapped} reviewed exclusions.")
        print("This does not establish feature, build, artifact, platform, or runtime test parity.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
