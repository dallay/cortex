#!/usr/bin/env bash

DOCTOR_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$DOCTOR_DIR/.." && pwd)"
PACKAGE_JSON="$ROOT_DIR/package.json"
RUST_TOOLCHAIN_FILE="$ROOT_DIR/rust-toolchain.toml"

read_node_minimum() {
    local manifest="${1:-$PACKAGE_JSON}"
    sed -nE '/"engines"[[:space:]]*:/,/^[[:space:]]*}/s/^[[:space:]]*"node"[[:space:]]*:[[:space:]]*">=([0-9]+(\.[0-9]+)?(\.[0-9]+)?).*/\1/p' "$manifest" | head -n 1
}

read_pnpm_version() {
    local manifest="${1:-$PACKAGE_JSON}"
    sed -nE 's/^[[:space:]]*"packageManager"[[:space:]]*:[[:space:]]*"pnpm@([^"]+)".*/\1/p' "$manifest" | head -n 1
}

read_pnpm_actual() {
    COREPACK_ENABLE_NETWORK=0 pnpm --version
}

check_pnpm_version() {
    local expected="$1"
    local actual

    if ! command -v pnpm >/dev/null 2>&1; then
        doctor_fail "pnpm $expected: 'pnpm' not found."
        return 1
    fi

    if actual="$(read_pnpm_actual 2>&1)"; then
        if [[ "$actual" == "$expected" ]]; then
            doctor_ok "pnpm $actual (pinned version in package.json)."
        else
            doctor_fail "pnpm $actual does not match pnpm $expected in package.json."
            return 1
        fi
    else
        actual="${actual%%$'\n'*}"
        doctor_fail "Could not verify pnpm $expected without Corepack network access: ${actual:-version not available}."
        return 1
    fi
}

read_rust_channel() {
    local manifest="${1:-$RUST_TOOLCHAIN_FILE}"
    sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$manifest" | head -n 1
}

read_rust_components() {
    local manifest="${1:-$RUST_TOOLCHAIN_FILE}"
    sed -nE 's/^[[:space:]]*components[[:space:]]*=[[:space:]]*\[(.*)\].*/\1/p' "$manifest" |
        sed -E 's/"//g; s/,[[:space:]]*/ /g'
}

version_at_least() {
    local actual="${1#v}"
    local required="${2#>=}"
    local actual_major actual_minor actual_patch
    local required_major required_minor required_patch

    IFS=. read -r actual_major actual_minor actual_patch <<< "$actual"
    IFS=. read -r required_major required_minor required_patch <<< "$required"

    actual_major="${actual_major%%[^0-9]*}"
    actual_minor="${actual_minor%%[^0-9]*}"
    actual_patch="${actual_patch%%[^0-9]*}"
    required_major="${required_major%%[^0-9]*}"
    required_minor="${required_minor%%[^0-9]*}"
    required_patch="${required_patch%%[^0-9]*}"

    [[ "$actual_major" =~ ^[0-9]+$ && "$required_major" =~ ^[0-9]+$ ]] || return 1
    actual_minor="${actual_minor:-0}"
    actual_patch="${actual_patch:-0}"
    required_minor="${required_minor:-0}"
    required_patch="${required_patch:-0}"

    if (( 10#$actual_major != 10#$required_major )); then
        (( 10#$actual_major > 10#$required_major ))
    elif (( 10#$actual_minor != 10#$required_minor )); then
        (( 10#$actual_minor > 10#$required_minor ))
    else
        (( 10#$actual_patch >= 10#$required_patch ))
    fi
}

doctor_ok() {
    printf '[OK] %s\n' "$1"
}

doctor_fail() {
    DOCTOR_FAILURES=$((DOCTOR_FAILURES + 1))
    printf '[MISSING] %s\n' "$1"
}

doctor_warn() {
    DOCTOR_WARNINGS=$((DOCTOR_WARNINGS + 1))
    printf '[WARNING] %s\n' "$1"
}

check_command() {
    local label="$1"
    local binary="$2"
    local version_arg="${3:---version}"
    local version

    if ! command -v "$binary" >/dev/null 2>&1; then
        doctor_fail "$label: '$binary' not found."
        return 1
    fi

    if ! version="$("$binary" "$version_arg" 2>&1)"; then
        version="${version%%$'\n'*}"
        doctor_fail "$label: '$binary $version_arg' failed: ${version:-no details}."
        return 1
    fi
    version="${version%%$'\n'*}"
    doctor_ok "$label: $version"
}

check_optional_command() {
    local binary="$1"
    local recipe="$2"

    if command -v "$binary" >/dev/null 2>&1; then
        doctor_ok "$binary is available."
    else
        doctor_warn "$binary missing; only needed for '$recipe'."
    fi
}

main() {
    RUSTUP_AUTO_INSTALL=0
    export RUSTUP_AUTO_INSTALL
    DOCTOR_FAILURES=0
    DOCTOR_WARNINGS=0
    local node_minimum pnpm_expected rust_channel rust_components component installed_components
    local node_actual

    printf 'Local Cortex Diagnostics (does not install or modify anything)\n\n'
    printf 'Base Tools\n'
    check_command "just" "just" || true
    check_command "Rustup" "rustup" || true
    check_command "Cargo" "cargo" || true
    check_command "Rustc" "rustc" || true
    check_command "Python 3" "python3" || true

    node_minimum="$(read_node_minimum)"
    if [[ -z "$node_minimum" ]]; then
        doctor_fail "Could not read Node.js minimum from package.json."
    elif ! command -v node >/dev/null 2>&1; then
        doctor_fail "Node.js >= $node_minimum: 'node' not found."
    else
        node_actual="$(node --version 2>&1)" || node_actual=""
        if version_at_least "$node_actual" "$node_minimum"; then
            doctor_ok "Node.js $node_actual (minimum required: >= $node_minimum)."
        else
            doctor_fail "Node.js $node_actual does not meet the minimum >= $node_minimum in package.json."
        fi
    fi

    pnpm_expected="$(read_pnpm_version)"
    if [[ -z "$pnpm_expected" ]]; then
        doctor_fail "Could not read packageManager from package.json."
    else
        check_pnpm_version "$pnpm_expected" || true
    fi

    printf '\nToolchain Rust\n'
    rust_channel="$(read_rust_channel)"
    if [[ -z "$rust_channel" ]]; then
        doctor_fail "No pude leer el canal Rust desde rust-toolchain.toml."
    elif command -v rustup >/dev/null 2>&1; then
        if rustup run "$rust_channel" rustc --version >/dev/null 2>&1; then
            doctor_ok "Rust toolchain '$rust_channel' is installed."
            rust_components="$(read_rust_components)"
            installed_components="$(rustup component list --installed --toolchain "$rust_channel" 2>/dev/null)" || installed_components=""
            for component in $rust_components; do
                if printf '%s\n' "$installed_components" | grep -Eq "^${component}(-|$)"; then
                    doctor_ok "Rust component '$component' is installed."
                else
                    doctor_fail "Missing Rust component '$component' from rust-toolchain.toml (install it with rustup)."
                fi
            done
        else
            doctor_fail "Rust toolchain '$rust_channel' from rust-toolchain.toml is not installed."
        fi
    else
        doctor_fail "Cannot verify Rust toolchain '$rust_channel' without rustup."
    fi

    printf '\nWorkspace\n'
    if [[ -f "$ROOT_DIR/pnpm-lock.yaml" && -f "$ROOT_DIR/pnpm-workspace.yaml" ]]; then
        doctor_ok "pnpm workspace and root lockfile are present."
    else
        doctor_fail "Missing pnpm-lock.yaml or pnpm-workspace.yaml in root; run installation from the root workspace."
    fi
    if [[ ! -e "$ROOT_DIR/apps/rook/dashboard/pnpm-lock.yaml" && ! -e "$ROOT_DIR/apps/rook/dashboard/pnpm-workspace.yaml" ]]; then
        doctor_ok "The dashboard uses the root pnpm workspace."
    else
        doctor_fail "The dashboard contains a local pnpm lockfile or workspace; CI requires a single root workspace."
    fi
    if [[ -d "$ROOT_DIR/apps/rook/dashboard/node_modules" ]]; then
        doctor_ok "Dashboard dependencies installed."
    else
        doctor_warn "Missing dashboard dependencies; run 'just setup'."
    fi

    printf '\nOptional Tools\n'
    if command -v docker >/dev/null 2>&1; then
        if docker info >/dev/null 2>&1; then
            doctor_ok "Docker is installed and daemon is responding."
        else
            doctor_warn "Docker is installed, but daemon is not responding; required for E2E and containers."
        fi
    else
        doctor_warn "Docker is not installed; required only for E2E and container recipes."
    fi

    check_optional_command "cargo-watch" "just dev"
    check_optional_command "cargo-llvm-cov" "just coverage"
    check_optional_command "cargo-audit" "just audit"
    check_optional_command "cargo-outdated" "just outdated"
    check_optional_command "cargo-udeps" "just unused"
    check_optional_command "cargo-hack" "just hack"
    check_optional_command "cargo-spellcheck" "just spellcheck"

    printf '\nSummary: %s mandatory requirement(s) missing, %s optional warning(s).\n' \
        "$DOCTOR_FAILURES" "$DOCTOR_WARNINGS"
    if (( DOCTOR_FAILURES > 0 )); then
        printf 'Mandatory requirements are missing. Review [MISSING] messages before continuing.\n'
        return 1
    fi

    printf 'Mandatory requirements ready. Optional warnings only affect the indicated recipes.\n'
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
