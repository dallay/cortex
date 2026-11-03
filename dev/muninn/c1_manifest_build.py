"""Compose a C1 freeze manifest that covers every execution-relevant input.

The earlier freeze manifest listed the corpus, labels, policy and the exporter
sources, but it did not list the whole build input of the Rust exporter: the
exporter package sources, the workspace manifests, the pinned toolchain and the
patched ``candle-core`` crate. A build record that cites a source outside the
manifest is not bound to the frozen inventory, so this module composes a
complete inventory and writes a new manifest without mutating the previous
one.

The module never runs inference and never touches Neo4j.
"""
import argparse
import json
import os
from pathlib import Path
import sys

import c1_freeze

DEFAULT_PRIOR = ".agents/rpi/goal/muninn-functional-parity/attempt67-freeze-manifest.json"

# Directories whose files are part of the Rust build input of the exporter.
SOURCE_TREES = (
    "crates/muninn/c1-exporter",
    "third_party/candle-core",
)

# Individual files that the build input requires.
SOURCE_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "dev/muninn/c1_runner.py",
    "dev/muninn/c1_execution.py",
    "dev/muninn/c1_freeze.py",
    "dev/muninn/c1_manifest_build.py",
    "dev/muninn/c1_provenance.py",
    "dev/muninn/c1_build_record.py",
    "dev/muninn/fixtures/minilm-c1-contract-cases.json",
    "dev/muninn/fixtures/minilm-c1-identity-golden.json",
    "dev/muninn/fixtures/artifact-lock.json",
    "dev/muninn/minilm_tokenizer_reference.py",
    "dev/muninn/requirements-minilm-tokenizer.txt",
    ".github/workflows/ci.yml",
    "dev/muninn/c1_attempt_run.py",
    "dev/muninn/c1_attempt_plan.json",
    "dev/muninn/c1_acceptance.py",
    "dev/muninn/tools/c1_python_exporter.sh",
    ".agents/rpi/goal/muninn-functional-parity/attempt68-bundle-verify.jsonl",
)

SKIP_DIRECTORIES = ("target", ".git", "__pycache__", "artifacts", ".cache")


class ManifestError(ValueError):
    """The requested inventory cannot be frozen."""


def _is_skipped(relative_parts):
    return any(part in SKIP_DIRECTORIES for part in relative_parts)


def _walk(root, relative):
    """Return every regular file under a workspace-relative directory.

    A symlinked file or directory inside the tree is not part of the frozen
    inventory: it can point outside the workspace, or be swapped after the
    freeze. Rather than silently omit it, the walk fails closed so the
    manifest author sees exactly why the source tree is not admitted.
    """
    base = Path(root) / relative
    if base.is_symlink() or not base.is_dir():
        raise ManifestError("missing_source_tree")
    found = []
    for current, directories, files in os.walk(base):
        kept = []
        for name in sorted(directories):
            if name in SKIP_DIRECTORIES:
                continue
            if (Path(current) / name).is_symlink():
                raise ManifestError("symlinked_source_tree",
                                     str(Path(current) / name))
            kept.append(name)
        directories[:] = kept
        for name in sorted(files):
            path = Path(current) / name
            if path.is_symlink():
                raise ManifestError("symlinked_source_file", str(path))
            found.append(path)
    return found


def collect_inputs(root, prior_manifest_path):
    """Return the absolute path of every input the next freeze must cover."""
    root = Path(root).resolve(strict=True)
    prior_path = Path(prior_manifest_path)
    if not prior_path.is_absolute():
        prior_path = root / prior_path
    try:
        prior = json.loads(prior_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        raise ManifestError("prior_manifest_unreadable") from None
    entries = prior.get("inputs")
    if type(entries) is not list or not entries:
        raise ManifestError("prior_manifest_inputs")
    paths = []
    seen = set()
    for entry in entries:
        if type(entry) is not dict:
            raise ManifestError("prior_manifest_entry")
        relative = entry.get("path")
        if type(relative) is not str or not relative:
            raise ManifestError("prior_manifest_entry")
        path = root / relative
        if not path.is_file() or path.is_symlink():
            raise ManifestError("prior_manifest_input_missing")
        if relative not in seen:
            seen.add(relative)
            paths.append(path)
    for relative in SOURCE_FILES:
        path = root / relative
        if not path.is_file() or path.is_symlink():
            raise ManifestError("missing_source_file")
        if relative not in seen:
            seen.add(relative)
            paths.append(path)
    for relative in SOURCE_TREES:
        for path in _walk(root, relative):
            marker = path.relative_to(root).as_posix()
            if marker in seen:
                continue
            seen.add(marker)
            paths.append(path)
    return sorted(paths, key=lambda item: item.relative_to(root).as_posix())


def build_manifest(root, prior_manifest_path, contract="minilm-cpu-f32-v1"):
    """Return a new manifest that supersedes the prior inventory."""
    root = Path(root).resolve(strict=True)
    inputs = collect_inputs(root, prior_manifest_path)
    return c1_freeze.build(root, inputs, contract)


def write_manifest(manifest, output_path):
    """Write the manifest to a fresh path; refuse to overwrite one."""
    output_path = Path(output_path)
    if output_path.exists():
        raise ManifestError("manifest_preexisting")
    output_path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    descriptor = os.open(str(output_path), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        stream.write(payload)
    return output_path


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    parser.add_argument("--prior", default=DEFAULT_PRIOR)
    parser.add_argument("--output", required=True)
    parser.add_argument("--contract", default="minilm-cpu-f32-v1")
    args = parser.parse_args(argv)
    try:
        manifest = build_manifest(args.root, args.prior, args.contract)
        path = write_manifest(manifest, args.output)
    except (ManifestError, c1_freeze.FreezeError) as error:
        print("manifest_build_failed: {0}".format(error), file=sys.stderr)
        return 1
    print("manifest_written: {0} ({1} inputs)".format(path, len(manifest["inputs"])))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
