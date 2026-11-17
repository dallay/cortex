"""Drive a C1 attempt from a frozen plan and a frozen manifest.

The plan describes what each stage executes. The driver resolves every source
of a stage from the manifest itself, so a stage can only cite bytes the
manifest admits. Missing admissions, a plan outside the manifest, or a plan
that does not match the manifest digest stop the attempt before any process
starts.

The driver never runs model inference itself, never touches Neo4j, and never
mutates the manifest, the corpus, the labels or the numeric policy.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

import c1_runner

PLAN_SCHEMA_VERSION = 1


class PlanError(ValueError):
    """The plan or the manifest does not support a safe attempt."""


def _sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_plan(plan_path):
    try:
        raw = Path(plan_path).read_bytes()
    except OSError:
        raise PlanError("plan_unreadable") from None
    try:
        plan = json.loads(raw.decode("utf-8"))
    except (ValueError, UnicodeError):
        raise PlanError("plan_json") from None
    if type(plan) is not dict or plan.get("schema_version") != PLAN_SCHEMA_VERSION:
        raise PlanError("plan_schema")
    stages = plan.get("stages")
    if type(stages) is not list or not stages:
        raise PlanError("plan_stages")
    return plan, raw


def manifest_entries(manifest):
    entries = {}
    for entry in manifest.get("inputs", []):
        if type(entry) is dict:
            entries[entry.get("path")] = entry
    return entries


def resolve_sources(root, entries, stage):
    """Return the absolute paths of every source the stage cites."""
    files = stage.get("files", [])
    trees = stage.get("source_trees", [])
    if type(files) is not list or type(trees) is not list:
        raise PlanError("stage_sources")
    selected = []
    for relative in files:
        if type(relative) is not str or relative not in entries:
            raise PlanError("source_not_admitted")
        selected.append(relative)
    for tree in trees:
        if type(tree) is not str:
            raise PlanError("stage_sources")
        prefix = tree if tree.endswith("/") else tree + "/"
        matches = [path for path in entries if path.startswith(prefix)]
        if not matches:
            raise PlanError("source_tree_empty")
        selected.extend(matches)
    ordered = []
    for relative in sorted(set(selected)):
        path = Path(root) / relative
        if not path.is_file():
            raise PlanError("source_missing")
        if _sha256_file(path) != entries[relative]["sha256"]:
            raise PlanError("source_digest_mismatch")
        ordered.append(path)
    return ordered


def build_stage(root, entries, stage):
    """Translate one plan stage into a runner stage."""
    if type(stage) is not dict:
        raise PlanError("stage_entry")
    name = stage.get("name")
    if type(name) is not str or not name:
        raise PlanError("stage_name")
    binary_relative = stage.get("binary")
    if type(binary_relative) is not str or not binary_relative:
        raise PlanError("stage_binary")
    binary = Path(root) / binary_relative
    sources = resolve_sources(root, entries, stage)
    inputs = tuple(c1_runner.InputBinding(role=item["role"], relative=item["relative"],
                                          delivery=item["delivery"])
                   for item in stage.get("inputs", []))
    consumed = tuple(c1_runner.ConsumptionCheck(output=item["output"], role=item["role"],
                                                field=item["field"])
                     for item in stage.get("consumed", []))
    argv = tuple(stage.get("argv", []))
    argv = tuple(str(binary) if argument == "{binary}" else argument for argument in argv)
    argv = tuple(argument.replace("{root}", str(root)) if type(argument) is str else argument
                 for argument in argv)
    toolchain = tuple(tuple(entry) for entry in stage.get("toolchain", []))
    env = tuple(tuple(entry) for entry in stage.get("env", []))
    provenance = ()
    if stage.get("provenance"):
        config = stage["provenance"]
        materials = [{"path": path.relative_to(Path(root)).as_posix(),
                      "sha256": entries[path.relative_to(Path(root)).as_posix()]["sha256"],
                      "size": entries[path.relative_to(Path(root)).as_posix()]["size"]}
                     for path in sources]
        record = Path(root) / config["record"]
        provenance = (record, Path(root) / config["bundle"], config["source_commit"],
                      materials, config["toolchain"])
        try:
            import c1_provenance
            authenticated = c1_provenance.verify(binary, *provenance)
        except (ValueError, OSError) as error:
            raise PlanError("build_provenance_rejected: " + str(error)) from error
        expected_digest = authenticated["binary_sha256"]
    elif binary.resolve() == Path(sys.executable).resolve():
        # Python stages bind their scripts through the frozen source inventory.
        expected_digest = _sha256_file(binary)
    else:
        raise PlanError("missing_build_provenance")
    return c1_runner.Stage(
        name=name,
        argv=argv,
        build=c1_runner.BuildBinding(
            binary=str(binary),
            binary_sha256=expected_digest,
            provenance=provenance,
            sources=tuple(c1_runner.SourceBinding(path=str(path),
                                                 sha256=entries[path.relative_to(
                                                     Path(root)).as_posix()]["sha256"])
                          for path in sources),
            toolchain=toolchain),
        inputs=inputs,
        outputs=tuple(stage.get("outputs", [])),
        consumed=consumed,
        output_dir=stage.get("output_dir", ""),
        timeout=float(stage.get("timeout", 900.0)),
        env=env)


def run_plan(root, manifest_path, manifest_sha256, plan_path, attempt_dir):
    """Run every planned stage against the frozen manifest."""
    root = Path(root).resolve(strict=True)
    plan, plan_raw = load_plan(plan_path)
    manifest_raw = Path(manifest_path).read_bytes()
    if hashlib.sha256(manifest_raw).hexdigest() != manifest_sha256:
        raise PlanError("manifest_digest")
    manifest = json.loads(manifest_raw.decode("utf-8"))
    entries = manifest_entries(manifest)
    plan_relative = Path(os.path.realpath(str(plan_path))).relative_to(
        Path(os.path.realpath(str(root)))).as_posix()
    if plan_relative not in entries:
        raise PlanError("plan_not_admitted")
    if entries[plan_relative]["sha256"] != hashlib.sha256(plan_raw).hexdigest():
        raise PlanError("plan_digest_mismatch")
    stages = [build_stage(root, entries, stage) for stage in plan["stages"]]
    return c1_runner.run_attempt(str(root), str(Path(manifest_path).resolve()), manifest_sha256,
                                 stages, attempt_dir)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--plan", required=True)
    parser.add_argument("--attempt-dir", required=True)
    args = parser.parse_args(argv)
    try:
        receipt = run_plan(args.root, args.manifest, args.manifest_sha256, args.plan,
                           args.attempt_dir)
    except (PlanError, c1_runner.RunnerError) as error:
        print("attempt_failed: {0}".format(error), file=sys.stderr)
        return 1
    print(json.dumps({"status": receipt.get("status"),
                      "receipt": str(Path(args.attempt_dir) / c1_runner.RECEIPT_NAME)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
