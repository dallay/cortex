#!/usr/bin/env bash
# Shared helpers for the dashboard Bazel targets.
#
# The dashboard toolchain is provided by pnpm (the root pnpm-lock.yaml stays
# authoritative), not by Bazel. These helpers locate the installed toolchain
# and the real source tree, so the targets execute the same commands a
# developer runs by hand.

# Resolves the dashboard source directory and cd into it.
dashboard_prepare_workspace() {
  local runfiles_dir="${TEST_SRCDIR:-}"
  if [[ -z "${runfiles_dir}" ]]; then
    echo "TEST_SRCDIR is not set; run this target with bazel test." >&2
    return 1
  fi

  local manifest="${runfiles_dir}/${TEST_WORKSPACE:-_main}/apps/rook/dashboard/package.json"
  if [[ ! -e "${manifest}" ]]; then
    echo "dashboard package.json is not available in the runfiles: ${manifest}" >&2
    return 1
  fi

  # The runfiles entry is a symlink to the source file, so the resolved path
  # points back to the source tree that holds node_modules.
  local dashboard_dir
  dashboard_dir="$(dirname "$(readlink "${manifest}")")"
  cd "${dashboard_dir}"

  if [[ ! -d node_modules ]]; then
    echo "dashboard node_modules is missing; run 'pnpm install' (or 'just dashboard-install') first." >&2
    return 1
  fi
}

# Puts the installed Node toolchain on PATH. Bazel runs tests with a minimal
# PATH, so the usual install locations are added when they exist, and a Node
# version new enough for the dashboard toolchain is selected explicitly.
dashboard_prepare_toolchain() {
  local candidate
  for candidate in \
    "${HOME}/Library/pnpm/bin" \
    "${HOME}/.local/share/pnpm" \
    "${HOME}/.local/pnpm" \
    "/opt/homebrew/bin" \
    "/usr/local/bin"; do
    if [[ -d "${candidate}" && ":${PATH}:" != *":${candidate}:"* ]]; then
      PATH="${PATH}:${candidate}"
    fi
  done
  export PATH

  dashboard_select_node

  if command -v pnpm >/dev/null 2>&1; then
    return 0
  fi

  if command -v corepack >/dev/null 2>&1; then
    echo "pnpm is not on PATH; using corepack pnpm." >&2
    return 0
  fi

  echo "pnpm is not available; install it before running the dashboard targets." >&2
  return 1
}

# Selects the newest Node runtime found in the common version-manager and
# system locations, and puts it first on PATH. The dashboard requires Node 24
# or newer, while a system Node can be several majors behind.
dashboard_select_node() {
  local best_node="" best_major=-1
  local node_path major

  local candidates=()
  local dir
  for dir in \
    "${HOME}/.local/state/fnm_multishells"/*/bin \
    "${HOME}/.fnm/node-versions"/*/installation/bin \
    "${HOME}/.nvm/versions/node"/*/bin \
    "${HOME}/.asdf/installs/nodejs"/*/bin \
    "${HOME}/.local/share/mise/installs/node"/*/bin; do
    [[ -d "${dir}" ]] && candidates+=("${dir}")
  done
  candidates+=("/opt/homebrew/bin" "/usr/local/bin" "${HOME}/.local/bin")

  for dir in "${candidates[@]}"; do
    node_path="${dir}/node"
    [[ -x "${node_path}" ]] || continue
    major="$("${node_path}" --version 2>/dev/null | sed 's/^v//' | cut -d. -f1)"
    [[ "${major}" =~ ^[0-9]+$ ]] || continue
    if (( major > best_major )); then
      best_major="${major}"
      best_node="${node_path}"
    fi
  done

  if [[ -n "${best_node}" && "${best_major}" -ge 20 ]]; then
    PATH="$(dirname "${best_node}"):${PATH}"
    export PATH
  fi
}
