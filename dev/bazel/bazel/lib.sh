#!/usr/bin/env bash
# Shared helpers for the Bazel targets that cover the BUILD file tooling.
#
# The Python toolchain is provided by the system, not by Bazel. These helpers
# locate an interpreter new enough for the tooling and the runfiles tree that
# holds the scripts under test, so the targets execute the same commands a
# developer runs by hand.

# Moves into the runfiles tree that holds the scripts under test.
bazel_prepare_workspace() {
  local runfiles_dir="${TEST_SRCDIR:-}"
  if [[ -z "${runfiles_dir}" ]]; then
    echo "TEST_SRCDIR is not set; run this target with bazel test." >&2
    return 1
  fi

  local workspace="${TEST_WORKSPACE:-_main}"
  if [[ ! -d "${runfiles_dir}/${workspace}/dev/bazel" ]]; then
    echo "dev/bazel is not available in the runfiles: ${runfiles_dir}/${workspace}/dev/bazel" >&2
    return 1
  fi

  cd "${runfiles_dir}/${workspace}"
}

# Puts a supported Python interpreter on PATH. Bazel runs tests with a minimal
# PATH, and the system interpreter bundled with the OS can be older than the
# version the tooling supports, so the candidates are inspected explicitly.
bazel_prepare_python() {
  local candidate
  for candidate in \
    "${HOME}/.pyenv/shims" \
    "${HOME}/.local/bin" \
    "/opt/homebrew/bin" \
    "/usr/local/bin"; do
    if [[ -d "${candidate}" && ":${PATH}:" != *":${candidate}:"* ]]; then
      PATH="${PATH}:${candidate}"
    fi
  done
  export PATH

  bazel_select_python
}

# Selects the first interpreter that reports a supported version and puts its
# directory first on PATH. The tooling needs Python 3.10 or newer.
bazel_select_python() {
  local candidates=()
  local dir
  for dir in \
    "${HOME}/.pyenv/versions"/*/bin \
    "${HOME}/.local/share/mise/installs/python"/*/bin; do
    [[ -d "${dir}" ]] && candidates+=("${dir}")
  done
  candidates+=("/opt/homebrew/bin" "/usr/local/bin" "${HOME}/.local/bin" "/usr/bin")

  local python_path version major minor
  for dir in "${candidates[@]}"; do
    python_path="${dir}/python3"
    [[ -x "${python_path}" ]] || continue
    version="$("${python_path}" --version 2>&1 | awk '{print $2}')"
    major="${version%%.*}"
    minor="${version#*.}"
    minor="${minor%%.*}"
    [[ "${major}" =~ ^[0-9]+$ && "${minor}" =~ ^[0-9]+$ ]] || continue
    if (( major > 3 )) || { (( major == 3 )) && (( minor >= 10 )); }; then
      PATH="${dir}:${PATH}"
      export PATH
      return 0
    fi
  done

  echo "no Python 3.10 or newer interpreter was found; the BUILD generator needs one." >&2
  return 1
}
