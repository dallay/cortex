#!/usr/bin/env python3
"""Deterministic disposable-filesystem probe for App Sandbox feasibility."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile

OPERATIONS = ("read", "list", "create", "modify", "replace")
ROLES = ("host", "helper", "shell", "nested")


def make_fixture(root: pathlib.Path) -> dict[str, pathlib.Path]:
    (root / ".probe-fixture").write_text("disposable probe fixture\n", encoding="utf-8")
    chosen = root / "chosen-workspace"
    unchosen = root / "unchosen-workspace"
    grants = root / "fake-home" / "Library" / "Application Support" / "Huginn" / "grants"
    for resource in (chosen, unchosen, grants):
        resource.mkdir(parents=True)
        (resource / "marker.txt").write_text("sentinel\n", encoding="utf-8")
    # Probe every directory that a future grant file would depend on.
    ancestors = []
    parent = grants.parent
    while parent != root.parent:
        ancestors.append(parent)
        if parent == root:
            break
        parent = parent.parent
    return {"chosen": chosen, "unchosen": unchosen, **{f"grant-ancestor-{i}": p for i, p in enumerate(ancestors)}, "grant-store": grants}


def resolve_probe_binary(candidate: str) -> pathlib.Path:
    """Resolve the probe binary and refuse anything that is not an existing file."""
    binary = pathlib.Path(candidate).expanduser().resolve()
    if not binary.is_file():
        raise ValueError(f"probe binary is not an existing file: {candidate}")
    return binary


def run_probe(binary: pathlib.Path, role: str, resource: pathlib.Path, operation: str) -> dict:
    completed = subprocess.run(
        [str(binary), "--probe", role, str(resource), operation],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"probe process failed ({role}, {resource}, {operation}): {completed.stderr}")
    return json.loads(completed.stdout)


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: harness.py PROBE_BINARY baseline|sandbox", file=sys.stderr)
        return 64
    binary = resolve_probe_binary(sys.argv[1])
    mode = sys.argv[2]
    if mode not in {"baseline", "sandbox"}:
        print("mode must be baseline or sandbox", file=sys.stderr)
        return 64

    with tempfile.TemporaryDirectory(prefix="huginn-app-sandbox-probe-") as temporary:
        root = pathlib.Path(temporary)
        resources = make_fixture(root)
        # Ancestor directory probes need an in-directory marker and writable controls.
        for path in resources.values():
            marker = path / "marker.txt"
            if not marker.exists():
                marker.write_text("sentinel\n", encoding="utf-8")

        failures = []
        count = 0
        for role in ROLES:
            for name, path in resources.items():
                for operation in OPERATIONS:
                    result = run_probe(binary, role, path, operation)
                    count += 1
                    allowed = name == "chosen"
                    expectation_met = result["succeeded"] if allowed else not result["succeeded"]
                    print(json.dumps({"expected": "allow" if allowed else "deny", **result}, sort_keys=True))
                    if not expectation_met:
                        failures.append((role, name, operation, result["succeeded"], result.get("error")))

        print(f"SUMMARY mode={mode} probes={count} expectation_failures={len(failures)}")
        for role, name, operation, succeeded, error in failures:
            print(f"MISMATCH role={role} resource={name} operation={operation} succeeded={succeeded} error={error}")
        # Baseline must be RED: outside access succeeding is a confirmed boundary failure.
        if mode == "baseline":
            outside_successes = sum(1 for failure in failures if failure[1] != "chosen")
            print(f"RED baseline outside_access_successes={outside_successes}; sandbox boundary absent")
            return 1 if failures else 0
        return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
