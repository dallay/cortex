#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCTOR_SCRIPT="$SCRIPT_DIR/doctor.sh"

# Source the public helpers so version and manifest behavior can be tested
# without requiring a complete developer toolchain.
# shellcheck source=dev/doctor.sh
source "$DOCTOR_SCRIPT"

assert_eq() {
    local expected="$1"
    local actual="$2"
    local name="$3"
    if [[ "$actual" != "$expected" ]]; then
        printf 'FAIL: %s (expected "%s", got "%s")\n' "$name" "$expected" "$actual" >&2
        exit 1
    fi
    printf 'PASS: %s\n' "$name"
}

assert_true() {
    local name="$1"
    shift
    if ! "$@"; then
        printf 'FAIL: %s\n' "$name" >&2
        exit 1
    fi
    printf 'PASS: %s\n' "$name"
}

assert_false() {
    local name="$1"
    shift
    if "$@"; then
        printf 'FAIL: %s\n' "$name" >&2
        exit 1
    fi
    printf 'PASS: %s\n' "$name"
}

assert_eq "24" "$(read_node_minimum)" "Node minimum comes from package.json"
assert_eq "12.15.0" "$(read_pnpm_version)" "pnpm version comes from package.json"
assert_eq "1.99" "$(read_rust_channel)" "Rust channel comes from rust-toolchain.toml"
assert_true "Node 24 satisfies the repository minimum" version_at_least "v24.0.0" "24"
assert_true "Newer Node satisfies the repository minimum" version_at_least "v24.22.1" "24"
assert_false "Node 23 does not satisfy the repository minimum" version_at_least "v23.99.9" "24"
assert_true "pnpm exact version comparison accepts a match" version_at_least "12.15.0" "12.15.0"
assert_false "pnpm exact version comparison rejects an older version" version_at_least "12.14.9" "12.15.0"

# Keep only basic shell utilities on PATH so required tool checks must report
# missing tools instead of accidentally finding the developer's installed tools.
restricted_path="$(mktemp -d)"
trap 'rm -rf "$restricted_path"' EXIT
for utility in dirname grep head sed; do
    utility_path="$(command -v "$utility")"
    ln -s "$utility_path" "$restricted_path/$utility"
done

set +e
missing_output="$(PATH="$restricted_path" /bin/bash "$DOCTOR_SCRIPT" 2>&1)"
missing_status=$?
set -e

if [[ "$missing_status" -eq 0 ]]; then
    printf 'FAIL: doctor must fail when required tools are missing\n' >&2
    exit 1
fi
case "$missing_output" in
    *"Node.js"*"pnpm"*"Faltan requisitos obligatorios"*)
        printf 'PASS: doctor reports missing tools and exits non-zero\n'
        ;;
    *)
        printf 'FAIL: doctor output did not report the missing required tools\n%s\n' "$missing_output" >&2
        exit 1
        ;;
esac

printf 'All doctor tests passed.\n'
