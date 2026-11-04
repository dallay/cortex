#!/usr/bin/env bash
# Pre-push secret scan (GitGuardian ggshield) with unambiguous outcomes.
#
# lefthook.yml delegates the pre-push scan here so that a scanner that could not
# run is never reported as a secret finding. Exit status contract:
#   0  the scan completed and found nothing, or policy does not require it
#   1  the scan completed and found secrets, or it could not complete
#
# CORTEX_SECRET_SCAN_REQUIRED=1 makes the scan mandatory: an absent ggshield
# then fails closed instead of silently skipping.
set -uo pipefail

findings='GitGuardian secret scan found potential secrets. Inspect findings before pushing.'
fix_hint="Run 'ggshield auth login' or set GITGUARDIAN_API_KEY, then push again."

scan_required() {
    case ${CORTEX_SECRET_SCAN_REQUIRED:-} in
    '' | 0 | false | no | off) return 1 ;;
    *) return 0 ;;
    esac
}

if ! command -v ggshield >/dev/null 2>&1; then
    if scan_required; then
        echo "ggshield is not installed, so the required pre-push secret scan did not run (CORTEX_SECRET_SCAN_REQUIRED). Install it with 'pip install ggshield'. ${fix_hint}" >&2
        exit 1
    fi
    echo 'ggshield not installed, skipping pre-push scan'
    exit 0
fi

# ggshield streams its own report; its exit status is the only outcome signal.
ggshield secret scan pre-push "$@"
status=$?

case $status in
0)
    exit 0
    ;;
1)
    # ggshield ExitCode.SCAN_FOUND_PROBLEMS: the scan completed with findings.
    echo "$findings" >&2
    exit 1
    ;;
3)
    # ggshield ExitCode.AUTHENTICATION_ERROR.
    echo "ggshield could not run the pre-push secret scan: GitGuardian authentication is missing or invalid, so no secrets were scanned. ${fix_hint}" >&2
    exit 1
    ;;
*)
    echo "ggshield could not run the pre-push secret scan (exit status ${status}), so no secrets were scanned. Resolve the ggshield error above, then push again." >&2
    exit 1
    ;;
esac
