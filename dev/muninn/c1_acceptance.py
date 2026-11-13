"""Evaluate a C1 attempt against the frozen numeric policy.

The driver links an attempt receipt to the frozen manifest and policy:

* the receipt must bind the same manifest digest the policy was frozen with;
* every export it reads must match the digest recorded in the receipt;
* the corpus, labels and policy bytes must match the manifest entries;
* the evaluator then applies the frozen policy without any tolerance change.

The driver never runs inference, never touches Neo4j, and never modifies the
corpus, the labels, the policy or the manifest. A numeric pass is reported as
a numeric pass; the evaluator deliberately does not call that acceptance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys

import c1_numeric_policy

HEX_SHA256 = re.compile(r"[0-9a-f]{64}")


class AcceptanceError(ValueError):
    """The attempt, the manifest or the policy do not support an evaluation."""


def _sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def manifest_entries(manifest):
    entries = {}
    for entry in manifest.get("inputs", []):
        if type(entry) is dict and type(entry.get("path")) is str:
            entries[entry["path"]] = entry
    return entries


def load_receipt(attempt_dir):
    receipt_path = Path(attempt_dir) / "attempt-receipt.json"
    try:
        raw = receipt_path.read_bytes()
    except OSError:
        raise AcceptanceError("receipt_unreadable") from None
    try:
        receipt = json.loads(raw.decode("utf-8"))
    except (ValueError, UnicodeError):
        raise AcceptanceError("receipt_json") from None
    if type(receipt) is not dict or receipt.get("status") != "complete":
        raise AcceptanceError("receipt_status")
    return receipt, hashlib.sha256(raw).hexdigest()


REQUIRED_STAGES = ("rust-export", "python-export")


def export_bytes(attempt_dir, receipt, stage_name):
    """Return the export bytes for one required stage.

    The log location is derived from the attempt directory and the fixed
    stage/stream names, never from a path the receipt supplies. A forged or
    altered receipt cannot redirect acceptance to attacker-controlled bytes.
    The receipt-recorded digest is still checked as an additional integrity
    signal, but it is never the source of the path.
    """
    stages = receipt.get("stages")
    if type(stages) is not list:
        raise AcceptanceError("receipt_stages")
    matches = [stage for stage in stages
               if type(stage) is dict and stage.get("name") == stage_name]
    if not matches:
        raise AcceptanceError("stage_missing")
    if len(matches) > 1:
        raise AcceptanceError("stage_duplicate")
    stage = matches[0]
    log_path = Path(attempt_dir) / "logs" / (stage_name + ".stdout.log")
    try:
        real_attempt = Path(attempt_dir).resolve(strict=True)
        real_log = log_path.resolve(strict=True)
    except OSError:
        raise AcceptanceError("export_unreadable") from None
    try:
        real_log.relative_to(real_attempt)
    except ValueError:
        raise AcceptanceError("export_path_escape") from None
    if log_path.is_symlink():
        raise AcceptanceError("export_symlink_refused")
    try:
        info = log_path.lstat()
    except OSError:
        raise AcceptanceError("export_unreadable") from None
    if not stat.S_ISREG(info.st_mode):
        raise AcceptanceError("export_invalid")
    try:
        data = log_path.read_bytes()
    except OSError:
        raise AcceptanceError("export_unreadable") from None
    recorded = None
    for log in stage.get("logs", []):
        if type(log) is dict and log.get("stream") == "stdout":
            recorded = log.get("sha256")
            break
    if recorded is None:
        raise AcceptanceError("export_missing")
    if hashlib.sha256(data).hexdigest() != recorded:
        raise AcceptanceError("export_digest_mismatch")
    return data


def verify_frozen(root, entries, relative, label):
    """Return the frozen bytes of an input and check them against the manifest."""
    if relative not in entries:
        raise AcceptanceError("frozen_input_not_admitted")
    path = Path(root) / relative
    try:
        data = path.read_bytes()
    except OSError:
        raise AcceptanceError("frozen_input_unreadable") from None
    if len(data) != entries[relative]["size"] or \
            hashlib.sha256(data).hexdigest() != entries[relative]["sha256"]:
        raise AcceptanceError("frozen_input_digest_mismatch")
    return data, {"path": relative, "size": entries[relative]["size"],
                  "sha256": entries[relative]["sha256"], "role": label}


def evaluate_attempt(root, manifest_path, manifest_sha256, attempt_dir, corpus_relative,
                     labels_relative, policy_relative, expected_policy_relative,
                     expected_policy_sha256):
    """Evaluate one attempt and return the acceptance artifact.

    The caller cannot choose any admitted policy at will: ``policy_relative``
    must equal the frozen ``expected_policy_relative``, and the policy bytes
    must match the frozen ``expected_policy_sha256``, both supplied by the
    caller from an out-of-band, independently reviewed freeze record rather
    than derived from the manifest or the receipt being evaluated.
    """
    if type(expected_policy_sha256) is not str or not HEX_SHA256.fullmatch(
            expected_policy_sha256):
        raise AcceptanceError("expected_policy_digest_argument")
    if policy_relative != expected_policy_relative:
        raise AcceptanceError("frozen_policy_path_mismatch")
    root = Path(root).resolve(strict=True)
    manifest_raw = Path(manifest_path).read_bytes()
    if hashlib.sha256(manifest_raw).hexdigest() != manifest_sha256:
        raise AcceptanceError("manifest_digest")
    manifest = json.loads(manifest_raw.decode("utf-8"))
    entries = manifest_entries(manifest)
    receipt, receipt_sha256 = load_receipt(attempt_dir)
    if receipt.get("manifest", {}).get("sha256") != manifest_sha256:
        raise AcceptanceError("receipt_manifest_mismatch")
    corpus_bytes, corpus_identity = verify_frozen(root, entries, corpus_relative, "corpus")
    labels_bytes, labels_identity = verify_frozen(root, entries, labels_relative, "labels")
    policy_bytes, policy_identity = verify_frozen(root, entries, policy_relative, "policy")
    if hashlib.sha256(policy_bytes).hexdigest() != expected_policy_sha256:
        raise AcceptanceError("frozen_policy_digest_mismatch")
    rust_bytes = export_bytes(attempt_dir, receipt, "rust-export")
    python_bytes = export_bytes(attempt_dir, receipt, "python-export")
    try:
        rust = json.loads(rust_bytes.decode("utf-8"))
        python = json.loads(python_bytes.decode("utf-8"))
        labels = json.loads(labels_bytes.decode("utf-8"))
        policy = json.loads(policy_bytes.decode("utf-8"))
        report = c1_numeric_policy.evaluate(corpus_bytes, rust, python, labels, policy)
    except (ValueError, UnicodeError) as error:
        raise AcceptanceError("evaluation_input:{0}".format(error)) from None
    numeric_pass = report.get("state") == "numeric_pass_not_acceptance"
    return {
        "schema_version": 1,
        "numeric_contract_version": manifest.get("numeric_contract_version"),
        "state": report.get("state"),
        "numeric_parity": "pass" if numeric_pass else "fail",
        "encoder_acceptance": "not_established_by_numeric_parity",
        "acceptance_note": ("The frozen policy establishes numerical parity between the Rust "
                            "candidate and the Python reference for the recorded corpus. It does "
                            "not by itself establish encoder acceptance; that also requires the "
                            "independent review of the attempt receipt."),
        "manifest": {"sha256": manifest_sha256, "path": str(manifest_path)},
        "receipt": {"sha256": receipt_sha256, "path": str(Path(attempt_dir) /
                                                           "attempt-receipt.json")},
        "policy": dict(policy_identity, frozen=True),
        "corpus": corpus_identity,
        "labels": labels_identity,
        "exports": {
            "rust": {"sha256": hashlib.sha256(rust_bytes).hexdigest(),
                     "bytes": len(rust_bytes)},
            "python": {"sha256": hashlib.sha256(python_bytes).hexdigest(),
                       "bytes": len(python_bytes)},
        },
        "evaluation": report,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".")
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--attempt-dir", required=True)
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--labels", required=True)
    parser.add_argument("--policy", required=True)
    parser.add_argument("--expected-policy-relative", required=True)
    parser.add_argument("--expected-policy-sha256", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args(argv)
    try:
        artifact = evaluate_attempt(args.root, args.manifest, args.manifest_sha256,
                                    args.attempt_dir, args.corpus, args.labels, args.policy,
                                    args.expected_policy_relative,
                                    args.expected_policy_sha256)
    except (AcceptanceError, OSError) as error:
        print("acceptance_failed: {0}".format(error), file=sys.stderr)
        return 1
    output = Path(args.output)
    if output.exists():
        print("acceptance_failed: output_preexisting", file=sys.stderr)
        return 1
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"state": artifact["state"], "output": str(output)}))
    return 0 if artifact["numeric_parity"] == "pass" else 2


if __name__ == "__main__":
    raise SystemExit(main())
