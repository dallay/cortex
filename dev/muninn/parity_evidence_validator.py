"""Validate Muninn parity closure docs, traceability sync, and acceptance receipts."""
import argparse
import json
from pathlib import Path
import re

import api_test_traceability as traceability

ROOT = Path(__file__).resolve().parents[2]
CAPABILITIES_DOC = "docs/muninn/specs/bolt-parity-capabilities.md"
CLOSURE_DOC = "docs/muninn/specs/bolt-parity-final-closure.md"
TRACEABILITY_JSON = "docs/muninn/specs/api-test-traceability.json"
TRACEABILITY_MD = "docs/muninn/specs/api-test-traceability.md"

CAPABILITY_ID_RE = re.compile(r"\bBOLT-\d{2}-\d{2}\b")
REPO_PATH_RE = re.compile(
    r"`((?:crates|apps|dev|docs|tests)/[A-Za-z0-9_./-]+\.(?:rs|py|md|json|csv|yaml|yml|toml))`"
)
COMMIT_SHA_RE = re.compile(r"^[0-9a-f]{40}$")

# Non-averaged release-gate thresholds (SDD Master Plan Rev. 2)
ELIGIBLE_API_OPS_TOTAL = 138
MIN_API_OPS_FOR_95 = 132
INDEPENDENT_CAPABILITIES_TOTAL = 92
MIN_CAPABILITIES_FOR_95 = 88
CRITICAL_CRITERIA_TOTAL = 27
SELECTED_TCK_TOTAL = 178

ALLOWED_EXECUTION_STATUSES = {"PASSED", "FAILED", "NOT_RUN", "SKIPPED", "IGNORED"}


class EvidenceError(ValueError):
    """Parity evidence or closure document failed validation."""


def load_capability_ids(root=ROOT):
    """Parse all official BOLT-XX-YY capability IDs from bolt-parity-capabilities.md."""
    path = root / CAPABILITIES_DOC
    text = path.read_text(encoding="utf-8")
    ids = set(CAPABILITY_ID_RE.findall(text))
    if not ids:
        raise EvidenceError(f"No BOLT capability IDs found in {CAPABILITIES_DOC}")
    return ids


def find_missing_references_in_text(markdown_text, root=ROOT):
    """Return sorted unique repo file paths cited in backticks that do not exist."""
    missing = set()
    for match in REPO_PATH_RE.finditer(markdown_text):
        rel = match.group(1)
        if not (root / rel).exists():
            missing.add(rel)
    return sorted(missing)


def find_missing_repo_references(markdown_path, root=ROOT):
    """Check a markdown file for broken backtick-quoted repository file references."""
    text = Path(markdown_path).read_text(encoding="utf-8")
    return find_missing_references_in_text(text, root)


def validate_receipt(receipt, valid_capability_ids, expected_candidate_commit, root=ROOT):
    """Validate one machine-readable capability acceptance receipt fail-closed."""
    if not isinstance(receipt, dict):
        raise EvidenceError("receipt must be a JSON object")

    cap_id = receipt.get("capability_id")
    if cap_id not in valid_capability_ids:
        raise EvidenceError(f"unknown capability_id: {cap_id!r}")

    candidate_commit = receipt.get("candidate_commit")
    if (
        not isinstance(candidate_commit, str)
        or not COMMIT_SHA_RE.match(candidate_commit)
        or candidate_commit != expected_candidate_commit
    ):
        raise EvidenceError(
            f"candidate_commit {candidate_commit!r} does not match expected {expected_candidate_commit!r}"
        )

    status = receipt.get("execution_status")
    if status not in ALLOWED_EXECUTION_STATUSES:
        raise EvidenceError(f"invalid execution_status: {status!r}")

    accepted = receipt.get("accepted")
    if not isinstance(accepted, bool):
        raise EvidenceError("accepted must be a boolean")

    if accepted and status != "PASSED":
        raise EvidenceError(
            f"accepted=true is forbidden when execution_status={status!r}"
        )

    evidence = receipt.get("evidence")
    if not isinstance(evidence, list):
        raise EvidenceError("evidence must be a list of repository paths or receipt refs")
    if accepted and not evidence:
        raise EvidenceError("accepted=true requires non-empty evidence")
    for item in evidence:
        if not isinstance(item, str) or not item.strip():
            raise EvidenceError("evidence entries must be non-empty strings")
        if item.startswith(("crates/", "apps/", "dev/", "docs/", "tests/")):
            if not (root / item).exists():
                raise EvidenceError(f"missing evidence path: {item}")


def validate_receipts(receipts, valid_capability_ids, expected_candidate_commit, root=ROOT):
    """Validate a collection of capability receipts, rejecting duplicates."""
    if not isinstance(receipts, list):
        raise EvidenceError("receipts must be a list")
    seen = set()
    for receipt in receipts:
        validate_receipt(receipt, valid_capability_ids, expected_candidate_commit, root)
        cap_id = receipt["capability_id"]
        if cap_id in seen:
            raise EvidenceError(f"duplicate receipt for capability_id: {cap_id}")
        seen.add(cap_id)


def evaluate_release_gates(
    verified_api_ops,
    verified_capabilities,
    verified_critical,
    passed_tck,
):
    """Evaluate all four non-averaged release gates simultaneously."""
    gates = {
        "api_operation_parity": verified_api_ops >= MIN_API_OPS_FOR_95,
        "independent_capability_parity": verified_capabilities >= MIN_CAPABILITIES_FOR_95,
        "critical_acceptance": verified_critical >= CRITICAL_CRITERIA_TOTAL,
        "selected_tck": passed_tck >= SELECTED_TCK_TOTAL,
    }
    return {
        "accepted": all(gates.values()),
        "gates": gates,
        "metrics": {
            "verified_api_ops": verified_api_ops,
            "eligible_api_ops": ELIGIBLE_API_OPS_TOTAL,
            "verified_capabilities": verified_capabilities,
            "independent_capabilities": INDEPENDENT_CAPABILITIES_TOTAL,
            "verified_critical": verified_critical,
            "critical_total": CRITICAL_CRITERIA_TOTAL,
            "passed_tck": passed_tck,
            "selected_tck": SELECTED_TCK_TOTAL,
        },
    }


def check_repository(root=ROOT):
    """Run offline repository-level consistency checks for SDD-1B (F-19, F-20)."""
    cap_ids = load_capability_ids(root)
    if len(cap_ids) != INDEPENDENT_CAPABILITIES_TOTAL:
        raise EvidenceError(
            f"Expected {INDEPENDENT_CAPABILITIES_TOTAL} BOLT capabilities, found {len(cap_ids)}"
        )

    closure_path = root / CLOSURE_DOC
    missing = find_missing_repo_references(closure_path, root)
    if missing:
        raise EvidenceError(
            f"{CLOSURE_DOC} references non-existent files: {', '.join(missing)}"
        )

    report = json.loads((root / TRACEABILITY_JSON).read_text(encoding="utf-8"))
    md_text = (root / TRACEABILITY_MD).read_text(encoding="utf-8")
    try:
        traceability.validate_markdown_sync(report, md_text)
    except traceability.TraceabilityError as error:
        raise EvidenceError(str(error)) from error

    return {
        "capabilities_cataloged": len(cap_ids),
        "closure_missing_references": 0,
        "traceability_synced": True,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Validate closure doc references and traceability synchronization",
    )
    args = parser.parse_args(argv)
    if not args.check:
        parser.error("--check is required")
    try:
        summary = check_repository(ROOT)
    except EvidenceError as error:
        parser.exit(1, f"Parity evidence check failed: {error}\n")
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
