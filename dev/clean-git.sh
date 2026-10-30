#!/usr/bin/env bash
# Clean up local Git state: stale worktrees, merged branches, remote-tracking
# refs, expired reflogs, and unreachable objects.
#
# This script never deletes unmerged work and never touches the remote.
# Unmerged branches are reported so the operator can review them.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
RESET='\033[0m'

info() { printf "${GREEN}%s${RESET}\n" "$1"; }
warn() { printf "${YELLOW}%s${RESET}\n" "$1"; }
fail() { printf "${RED}%s${RESET}\n" "$1" >&2; }

PROTECTED_BRANCHES=("main" "master" "develop" "dev" "release")

is_protected() {
    local name="$1"
    local protected
    for protected in "${PROTECTED_BRANCHES[@]}"; do
        [[ "$name" == "$protected" ]] && return 0
    done
    return 1
}

require_clean_tree() {
    if ! git diff --quiet || ! git diff --cached --quiet || [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
        fail "El working tree tiene cambios sin commitear. Haz commit o stash antes de limpiar."
        exit 1
    fi
}

cleanup_worktrees() {
    info "==> Limpiando worktrees obsoletos"
    local removed=0
    while IFS= read -r worktree_path; do
        [[ -z "$worktree_path" ]] && continue
        if git -C "$worktree_path" diff --quiet 2>/dev/null && git -C "$worktree_path" diff --cached --quiet 2>/dev/null; then
            git worktree remove "$worktree_path" 2>/dev/null && removed=$((removed + 1))
        else
            warn "    omitido (tiene cambios): $worktree_path"
        fi
    done < <(git worktree list --porcelain | awk '/^worktree /{print $2}' | grep -v "^$ROOT_DIR$")
    git worktree prune
    info "    worktrees eliminados: $removed"
}

cleanup_merged_branches() {
    info "==> Eliminando ramas locales ya mergeadas"
    local removed=0
    local branch
    while IFS= read -r branch; do
        [[ -z "$branch" ]] && continue
        is_protected "$branch" && continue
        # -d refuses to delete unmerged branches, so unmerged work is never lost.
        if git branch -d "$branch" >/dev/null 2>&1; then
            removed=$((removed + 1))
        fi
    done < <(git branch --format='%(refname:short)' --merged)
    info "    ramas eliminadas: $removed"
}

report_unmerged_branches() {
    local unmerged
    unmerged="$(git branch --format='%(refname:short)' --no-merged | grep -v '^main$' || true)"
    if [[ -n "$unmerged" ]]; then
        warn "==> Ramas con trabajo sin mergear (NO se tocaron):"
        echo "$unmerged" | sed 's/^/    /'
    fi
}

cleanup_remote_refs() {
    info "==> Podando referencias remotas obsoletas"
    git fetch --prune --quiet origin 2>/dev/null || warn "    no se pudo hacer fetch --prune (sin red?)"
    local stale
    stale="$(git branch -r | grep -c '\[gone\]' || true)"
    info "    referencias remotas obsoletas restantes: ${stale:-0}"
}

cleanup_git_objects() {
    info "==> Limpiando reflogs y objetos inalcanzables"
    # Expire reflogs per-ref instead of using --all. `--all` also expires
    # refs/stash, which drops the stash stack and makes `git gc --prune=now`
    # delete the stashed commits. Stashes are user work-in-progress, so they
    # are never touched here.
    local ref
    while IFS= read -r ref; do
        [[ -z "$ref" ]] && continue
        git reflog expire --expire=30.days --expire-unreachable=now "$ref"
    done < <(git for-each-ref --format='%(refname)' refs/heads refs/remotes refs/tags)

    # HEAD reflog is tracked separately from the refs above.
    git reflog expire --expire=30.days --expire-unreachable=now HEAD

    # Protect the stash stack from the prune below. refs/stash points to the
    # newest stash, but `git stash list` reads the refs/stash reflog. If that
    # reflog is missing, recreate it so the stash stays visible.
    # The ref must be deleted and recreated because `git update-ref` with an
    # unchanged value does not write a reflog entry.
    if git rev-parse --verify --quiet refs/stash >/dev/null && [[ -z "$(git stash list)" ]]; then
        local stash_commit
        stash_commit="$(git rev-parse refs/stash)"
        git update-ref -d refs/stash
        git update-ref --create-reflog -m "clean-git: restore stash reflog" "refs/stash" "$stash_commit"
        warn "    reflog de refs/stash restaurado"
    fi

    git gc --prune=now --quiet
    info "    gc completado"
}

main() {
    require_clean_tree
    cleanup_worktrees
    cleanup_merged_branches
    report_unmerged_branches
    cleanup_remote_refs
    cleanup_git_objects
    info ""
    info "Limpieza de Git completada."
    git worktree list
}

main "$@"
