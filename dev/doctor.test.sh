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

test_root="$(mktemp -d)"
fixture_root="$test_root/fixtures"
restricted_path="$test_root/bin"
mkdir -p "$fixture_root" "$restricted_path"
trap 'rm -rf "$test_root"' EXIT

fixture_package="$fixture_root/package.json"
cat > "$fixture_package" <<'JSON'
{
  "packageManager": "pnpm@6.5.4",
  "engines": {
    "node": ">=18.2.3"
  }
}
JSON
fixture_toolchain="$fixture_root/rust-toolchain.toml"
cat > "$fixture_toolchain" <<'TOML'
[toolchain]
channel = "1.72.3"
components = ["rustfmt", "rustdoc"]
TOML

assert_eq "18.2.3" "$(read_node_minimum "$fixture_package")" "Node parser reads a controlled fixture"
assert_eq "6.5.4" "$(read_pnpm_version "$fixture_package")" "pnpm parser reads a controlled fixture"
assert_eq "1.72.3" "$(read_rust_channel "$fixture_toolchain")" "Rust parser reads a controlled fixture"
assert_eq "rustfmt rustdoc" "$(read_rust_components "$fixture_toolchain")" "Rust components parse from a controlled fixture"
assert_true "repository package.json exposes a Node requirement" test -n "$(read_node_minimum)"
assert_true "repository package.json exposes a pnpm pin" test -n "$(read_pnpm_version)"
assert_true "repository rust-toolchain.toml exposes a channel" test -n "$(read_rust_channel)"
assert_true "repository rust-toolchain.toml exposes components" test -n "$(read_rust_components)"
assert_true "a version above the fixture minimum is accepted" version_at_least "v18.3.0" "18.2.3"
assert_false "a version below the fixture minimum is rejected" version_at_least "v18.2.2" "18.2.3"
assert_true "an exact fixture version is accepted" version_at_least "6.5.4" "6.5.4"
assert_false "an older fixture version is rejected" version_at_least "6.5.3" "6.5.4"

# Keep only basic shell utilities on PATH so required tool checks must report
# missing tools instead of accidentally finding the developer's installed tools.
for utility in dirname grep head sed; do
    utility_path="$(command -v "$utility")"
    ln -s "$utility_path" "$restricted_path/$utility"
done

cat > "$restricted_path/pnpm" <<'SH'
#!/bin/sh
case "${PNPM_STUB_MODE:-network}" in
    network)
        printf '%s\n' "${COREPACK_ENABLE_NETWORK:-unset}"
        ;;
    version)
        printf '%s\n' "${PNPM_STUB_VERSION:-unset}"
        ;;
    unavailable)
        printf 'Network access disabled\n' >&2
        exit 42
        ;;
esac
SH
chmod +x "$restricted_path/pnpm"
pnpm_network_mode="$(PATH="$restricted_path:$PATH" read_pnpm_actual)"
assert_eq "0" "$pnpm_network_mode" "pnpm version probe disables Corepack network access"

DOCTOR_FAILURES=0
PATH="$restricted_path:$PATH" PNPM_STUB_MODE=version PNPM_STUB_VERSION="$(read_pnpm_version "$fixture_package")" \
    check_pnpm_version "$(read_pnpm_version "$fixture_package")" > "$restricted_path/pnpm-version.out" 2>&1
assert_eq "0" "$DOCTOR_FAILURES" "a cached pnpm version passes the read-only check"

DOCTOR_FAILURES=0
set +e
PATH="$restricted_path:$PATH" PNPM_STUB_MODE=unavailable \
    check_pnpm_version "$(read_pnpm_version "$fixture_package")" > "$restricted_path/pnpm-unavailable.out" 2>&1
pnpm_unavailable_status=$?
set -e
pnpm_unavailable_output="$(cat "$restricted_path/pnpm-unavailable.out")"
assert_eq "1" "$pnpm_unavailable_status" "an unavailable pinned pnpm version fails the check"
assert_eq "1" "$DOCTOR_FAILURES" "an unavailable pinned pnpm version increments failures"
case "$pnpm_unavailable_output" in
    *"No se pudo verificar pnpm $(read_pnpm_version "$fixture_package")"*"Network access disabled"*)
        printf 'PASS: unavailable pnpm reports the blocked Corepack download\n'
        ;;
    *)
        printf 'FAIL: unavailable pnpm did not report the reason\n%s\n' "$pnpm_unavailable_output" >&2
        exit 1
        ;;
esac

cat > "$restricted_path/rustup-proxy" <<'SH'
#!/bin/sh
if [ "${RUSTUP_AUTO_INSTALL:-}" != "0" ]; then
    printf '%s %s\n' "$0" "$*" >> "$RUSTUP_AUTO_INSTALL_MARKER"
    exit 99
fi
case "$1" in
    --version)
        printf 'rustup test proxy\n'
        ;;
    run)
        printf 'toolchain unavailable\n' >&2
        exit 1
        ;;
esac
SH
chmod +x "$restricted_path/rustup-proxy"
for proxy in rustup cargo rustc; do
    ln -s "$restricted_path/rustup-proxy" "$restricted_path/$proxy"
done

set +e
/usr/bin/env -u RUSTUP_AUTO_INSTALL \
    PATH="$restricted_path" \
    RUSTUP_AUTO_INSTALL_MARKER="$test_root/rustup-install-attempted" \
    /bin/bash "$DOCTOR_SCRIPT" > "$restricted_path/rustup-simulation.out" 2>&1
rustup_simulation_status=$?
set -e
if [[ "$rustup_simulation_status" -eq 0 ]]; then
    printf 'FAIL: doctor should report a missing Rust toolchain in the simulation\n' >&2
    exit 1
fi
if [[ -e "$test_root/rustup-install-attempted" ]]; then
    printf 'FAIL: doctor attempted to install an unavailable Rust toolchain\n' >&2
    exit 1
fi
printf 'PASS: doctor does not trigger rustup auto-install for an unavailable toolchain\n'

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
