#!/usr/bin/env bash
# Regression tests for scripts/pre-push-secret-scan.sh (DALLAY-744).
#
# Contract pinned here:
#   * a scan that completed with findings blocks with the current text;
#   * a scanner that could not run fails without claiming secrets were found;
#   * an absent scanner fails closed only when the scan is required by policy.
set -uo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
wrapper="$script_dir/pre-push-secret-scan.sh"
# Resolve bash before the cases narrow PATH to a fixture directory.
bash_bin=$(command -v bash)
work=$(mktemp -d "${TMPDIR:-/tmp}/pre-push-secret-scan-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

failures=0
pass() { echo "ok - $1"; }
fail() {
    echo "not ok - $1" >&2
    failures=$((failures + 1))
}

findings_text='GitGuardian secret scan found potential secrets. Inspect findings before pushing.'
skip_text='ggshield not installed, skipping pre-push scan'

# --- fixtures ---------------------------------------------------------------
fake_bin="$work/fake-bin"
empty_bin="$work/empty-bin"
mkdir -p "$fake_bin" "$empty_bin"

cat >"$fake_bin/ggshield" <<'FAKE'
#!/usr/bin/env bash
printf '%s\n' "$@" >"$FAKE_GGSHIELD_ARGS"
printf 'fake ggshield ran\n'
exit "${FAKE_GGSHIELD_EXIT:-0}"
FAKE
# Cases narrow PATH to the fixture directory, so the fixture needs an absolute
# interpreter and must not rely on looking up bash itself.
sed "1s|#!.*|#!${bash_bin}|" "$fake_bin/ggshield" >"$fake_bin/ggshield.tmp"
mv "$fake_bin/ggshield.tmp" "$fake_bin/ggshield"
chmod +x "$fake_bin/ggshield"

args_file="$work/args"
stdout_file="$work/stdout"
stderr_file="$work/stderr"

# run_wrapper <path-dir> <fake-exit> <required|__unset__>
run_wrapper() {
    local path_dir=$1 fake_exit=$2 required=$3
    : >"$args_file"
    : >"$stdout_file"
    : >"$stderr_file"
    if [ "$required" = "__unset__" ]; then
        env -u CORTEX_SECRET_SCAN_REQUIRED \
            PATH="$path_dir" \
            FAKE_GGSHIELD_ARGS="$args_file" \
            FAKE_GGSHIELD_EXIT="$fake_exit" \
            "$bash_bin" "$wrapper" origin git@example.com:repo.git \
            >"$stdout_file" 2>"$stderr_file"
    else
        env CORTEX_SECRET_SCAN_REQUIRED="$required" \
            PATH="$path_dir" \
            FAKE_GGSHIELD_ARGS="$args_file" \
            FAKE_GGSHIELD_EXIT="$fake_exit" \
            "$bash_bin" "$wrapper" origin git@example.com:repo.git \
            >"$stdout_file" 2>"$stderr_file"
    fi
    status=$?
}

expect_status() { # <label> <expected>
    if [ "$status" -eq "$2" ]; then
        pass "$1: exit status $2"
    else
        fail "$1: expected exit status $2, got $status"
    fi
}

expect_stderr_is() { # <label> <expected>
    local actual
    actual=$(cat "$stderr_file")
    if [ "$actual" = "$2" ]; then
        pass "$1: exact diagnostic"
    else
        fail "$1: expected stderr '$2', got '$actual'"
    fi
}

expect_stderr_contains() { # <label> <needle>
    if grep -Fq -- "$2" "$stderr_file"; then
        pass "$1: diagnostic mentions '$2'"
    else
        fail "$1: stderr '$(cat "$stderr_file")' does not mention '$2'"
    fi
}

expect_no_findings_claim() { # <label>
    if grep -Fq 'found potential secrets' "$stderr_file" "$stdout_file"; then
        fail "$1: reported a secret finding when the scan did not complete"
    else
        pass "$1: does not claim secrets were found"
    fi
}

# --- a completed scan with a clean result passes ----------------------------
run_wrapper "$fake_bin" 0 "__unset__"
expect_status "clean scan" 0
if [ -s "$stderr_file" ]; then
    fail "clean scan: expected empty stderr, got '$(cat "$stderr_file")'"
else
    pass "clean scan: no diagnostic"
fi
expected_args=$'secret\nscan\npre-push\norigin\ngit@example.com:repo.git'
if [ "$(cat "$args_file")" = "$expected_args" ]; then
    pass "clean scan: forwards ref arguments to ggshield unmodified"
else
    fail "clean scan: forwarded arguments were '$(tr '\n' ' ' <"$args_file")'"
fi

# --- a completed scan with findings blocks with the current text ------------
run_wrapper "$fake_bin" 1 "__unset__"
expect_status "findings" 1
expect_stderr_is "findings" "$findings_text"

# --- an unauthenticated scanner is not a finding ----------------------------
run_wrapper "$fake_bin" 3 "__unset__"
expect_status "unauthenticated" 1
expect_stderr_contains "unauthenticated" 'could not run the pre-push secret scan'
expect_stderr_contains "unauthenticated" 'ggshield auth login'
expect_stderr_contains "unauthenticated" 'GITGUARDIAN_API_KEY'
expect_no_findings_claim "unauthenticated"

# --- any other scanner failure is not a finding either ----------------------
run_wrapper "$fake_bin" 4 "__unset__"
expect_status "scanner unavailable" 1
expect_stderr_contains "scanner unavailable" 'could not run the pre-push secret scan'
expect_stderr_contains "scanner unavailable" 'exit status 4'
expect_no_findings_claim "scanner unavailable"

run_wrapper "$fake_bin" 2 "__unset__"
expect_status "usage error" 1
expect_no_findings_claim "usage error"

# --- an absent scanner skips when policy does not require the scan ----------
run_wrapper "$empty_bin" 0 "__unset__"
expect_status "absent, not required" 0
if [ "$(cat "$stdout_file")" = "$skip_text" ]; then
    pass "absent, not required: announces the skip"
else
    fail "absent, not required: expected '$skip_text', got '$(cat "$stdout_file")'"
fi

run_wrapper "$empty_bin" 0 "0"
expect_status "absent, CORTEX_SECRET_SCAN_REQUIRED=0" 0

# --- an absent scanner fails closed when policy requires the scan -----------
run_wrapper "$empty_bin" 0 "1"
expect_status "absent, required" 1
expect_stderr_contains "absent, required" 'did not run'
expect_stderr_contains "absent, required" 'ggshield auth login'
expect_stderr_contains "absent, required" 'GITGUARDIAN_API_KEY'
expect_no_findings_claim "absent, required"

run_wrapper "$empty_bin" 0 "true"
expect_status "absent, required by non-zero value" 1

if [ "$failures" -ne 0 ]; then
    echo "$failures assertion(s) failed" >&2
    exit 1
fi
echo "all pre-push secret scan assertions passed"
