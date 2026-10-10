#!/usr/bin/env bash
# Huginn — Linux daily-use smoke check.
#
# Verifies that the local Huginn install can build, run the deterministic suite
# and complete a read-only mock session on a clean Linux host. Does NOT contact
# a real model endpoint — that is the job of the daily-use checklist at
# docs/huginn/linux-daily-use.md.
#
# Usage:
#   ./scripts/huginn-linux-smoke.sh
#
# Writes a one-line record to docs/huginn/linux-smoke.log on success. Failure
# exits non-zero without touching the log.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

LOG_FILE="$REPO_ROOT/docs/huginn/linux-smoke.log"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
HOST="$(uname -n 2>/dev/null || echo unknown)"

note() { printf '==> %s\n' "$*"; }
die()  { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

# 1. Toolchain — Rust 1.89 is the floor declared in rust-toolchain.toml.
note "checking toolchain"
command -v rustc >/dev/null 2>&1 || die "rustc not on PATH"
command -v cargo  >/dev/null 2>&1 || die "cargo not on PATH"
command -v just   >/dev/null 2>&1 || die "just not on PATH (cargo install just)"

RUSTC_VERSION="$(rustc --version | awk '{print $2}')"
case "$RUSTC_VERSION" in
  1.89.*|1.9[0-9].*|2.*) ;; # 1.89 floor and any later stable.
  *) die "Rust 1.89+ required, found $RUSTC_VERSION" ;;
esac

# 2. Compile check — fails fast if the toolchain or workspace is broken.
note "cargo check -p huginn"
cargo check -p huginn --quiet

# 3. Deterministic suite — the one defined in justfile.
note "just huginn-test"
just huginn-test

# 4. Read-only mock session over a throwaway workspace. Uses --provider mock so
#    no network is required, and the 'list files' canned turn exercises the
#    tool registry path end-to-end without producing effects.
TMP_WORKSPACE="$(mktemp -d -t huginn-smoke-XXXXXX)"
trap 'rm -rf "$TMP_WORKSPACE"' EXIT

note "offline mock session"
cargo run -p huginn --quiet -- \
  --provider mock \
  --workspace "$TMP_WORKSPACE" \
  run "list files" >/dev/null

# 5. Append a success record. Only reached if every step above passed.
mkdir -p "$(dirname "$LOG_FILE")"
printf '%s host=%s rustc=%s result=ok\n' \
  "$TIMESTAMP" "$HOST" "$RUSTC_VERSION" >> "$LOG_FILE"

note "ok — record appended to ${LOG_FILE#"$REPO_ROOT"/}"
