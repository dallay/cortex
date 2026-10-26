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
        doctor_fail "pnpm $expected: no se encontró 'pnpm'."
        return 1
    fi

    if actual="$(read_pnpm_actual 2>&1)"; then
        if [[ "$actual" == "$expected" ]]; then
            doctor_ok "pnpm $actual (versión fijada en package.json)."
        else
            doctor_fail "pnpm $actual no coincide con pnpm $expected de package.json."
            return 1
        fi
    else
        actual="${actual%%$'\n'*}"
        doctor_fail "No se pudo verificar pnpm $expected sin acceso de red de Corepack: ${actual:-versión no disponible}."
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
    printf '[FALTA] %s\n' "$1"
}

doctor_warn() {
    DOCTOR_WARNINGS=$((DOCTOR_WARNINGS + 1))
    printf '[AVISO] %s\n' "$1"
}

check_command() {
    local label="$1"
    local binary="$2"
    local version_arg="${3:---version}"
    local version

    if ! command -v "$binary" >/dev/null 2>&1; then
        doctor_fail "$label: no se encontró '$binary'."
        return 1
    fi

    if ! version="$("$binary" "$version_arg" 2>&1)"; then
        version="${version%%$'\n'*}"
        doctor_fail "$label: falló '$binary $version_arg': ${version:-sin detalles}."
        return 1
    fi
    version="${version%%$'\n'*}"
    doctor_ok "$label: $version"
}

check_optional_command() {
    local binary="$1"
    local recipe="$2"

    if command -v "$binary" >/dev/null 2>&1; then
        doctor_ok "$binary está disponible."
    else
        doctor_warn "$binary falta; solo se necesita para '$recipe'."
    fi
}

main() {
    RUSTUP_AUTO_INSTALL=0
    export RUSTUP_AUTO_INSTALL
    DOCTOR_FAILURES=0
    DOCTOR_WARNINGS=0
    local node_minimum pnpm_expected rust_channel rust_components component installed_components
    local node_actual

    printf 'Diagnóstico local de Cortex (no instala ni modifica nada)\n\n'
    printf 'Herramientas base\n'
    check_command "just" "just" || true
    check_command "Rustup" "rustup" || true
    check_command "Cargo" "cargo" || true
    check_command "Rustc" "rustc" || true
    check_command "Python 3" "python3" || true

    node_minimum="$(read_node_minimum)"
    if [[ -z "$node_minimum" ]]; then
        doctor_fail "No pude leer el mínimo de Node.js desde package.json."
    elif ! command -v node >/dev/null 2>&1; then
        doctor_fail "Node.js >= $node_minimum: no se encontró 'node'."
    else
        node_actual="$(node --version 2>&1)" || node_actual=""
        if version_at_least "$node_actual" "$node_minimum"; then
            doctor_ok "Node.js $node_actual (mínimo requerido: >= $node_minimum)."
        else
            doctor_fail "Node.js $node_actual no cumple el mínimo >= $node_minimum de package.json."
        fi
    fi

    pnpm_expected="$(read_pnpm_version)"
    if [[ -z "$pnpm_expected" ]]; then
        doctor_fail "No pude leer packageManager desde package.json."
    else
        check_pnpm_version "$pnpm_expected" || true
    fi

    printf '\nToolchain Rust\n'
    rust_channel="$(read_rust_channel)"
    if [[ -z "$rust_channel" ]]; then
        doctor_fail "No pude leer el canal Rust desde rust-toolchain.toml."
    elif command -v rustup >/dev/null 2>&1; then
        if rustup run "$rust_channel" rustc --version >/dev/null 2>&1; then
            doctor_ok "Toolchain Rust '$rust_channel' está instalada."
            rust_components="$(read_rust_components)"
            installed_components="$(rustup component list --installed --toolchain "$rust_channel" 2>/dev/null)" || installed_components=""
            for component in $rust_components; do
                if printf '%s\n' "$installed_components" | grep -Eq "^${component}(-|$)"; then
                    doctor_ok "Componente Rust '$component' está instalado."
                else
                    doctor_fail "Falta el componente Rust '$component' de rust-toolchain.toml (instálalo con rustup)."
                fi
            done
        else
            doctor_fail "No está instalada la toolchain Rust '$rust_channel' indicada en rust-toolchain.toml."
        fi
    else
        doctor_fail "No puedo verificar la toolchain Rust '$rust_channel' sin rustup."
    fi

    printf '\nWorkspace\n'
    if [[ -f "$ROOT_DIR/pnpm-lock.yaml" && -f "$ROOT_DIR/pnpm-workspace.yaml" ]]; then
        doctor_ok "Workspace pnpm y lockfile de raíz están presentes."
    else
        doctor_fail "Falta pnpm-lock.yaml o pnpm-workspace.yaml en la raíz; ejecuta la instalación desde el workspace raíz."
    fi
    if [[ ! -e "$ROOT_DIR/apps/rook/dashboard/pnpm-lock.yaml" && ! -e "$ROOT_DIR/apps/rook/dashboard/pnpm-workspace.yaml" ]]; then
        doctor_ok "El dashboard usa el workspace pnpm de la raíz."
    else
        doctor_fail "El dashboard contiene un lockfile o workspace pnpm local; CI requiere un único workspace raíz."
    fi
    if [[ -d "$ROOT_DIR/apps/rook/dashboard/node_modules" ]]; then
        doctor_ok "Dependencias del dashboard instaladas."
    else
        doctor_warn "Faltan dependencias del dashboard; ejecuta 'just setup'."
    fi

    printf '\nHerramientas opcionales\n'
    if command -v docker >/dev/null 2>&1; then
        if docker info >/dev/null 2>&1; then
            doctor_ok "Docker está instalado y el daemon responde."
        else
            doctor_warn "Docker está instalado, pero el daemon no responde; se requiere para E2E y contenedores."
        fi
    else
        doctor_warn "Docker no está instalado; se requiere solo para E2E y recetas de contenedores."
    fi

    check_optional_command "cargo-watch" "just dev"
    check_optional_command "cargo-llvm-cov" "just coverage"
    check_optional_command "cargo-audit" "just audit"
    check_optional_command "cargo-outdated" "just outdated"
    check_optional_command "cargo-udeps" "just unused"
    check_optional_command "cargo-hack" "just hack"
    check_optional_command "cargo-spellcheck" "just spellcheck"

    printf '\nResumen: %s requisito(s) obligatorio(s) faltante(s), %s aviso(s) opcional(es).\n' \
        "$DOCTOR_FAILURES" "$DOCTOR_WARNINGS"
    if (( DOCTOR_FAILURES > 0 )); then
        printf 'Faltan requisitos obligatorios. Revisa los mensajes [FALTA] antes de continuar.\n'
        return 1
    fi

    printf 'Requisitos obligatorios listos. Los avisos opcionales solo afectan las recetas indicadas.\n'
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
