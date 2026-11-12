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
        fail "The working tree has uncommitted changes. Commit or stash before cleaning."
        exit 1
    fi
}

cleanup_worktrees() {
    info "==> Cleaning stale worktrees"
    local removed=0
    while IFS= read -r worktree_path; do
        [[ -z "$worktree_path" ]] && continue
        if git -C "$worktree_path" diff --quiet 2>/dev/null && git -C "$worktree_path" diff --cached --quiet 2>/dev/null; then
            git worktree remove "$worktree_path" 2>/dev/null && removed=$((removed + 1))
        else
            warn "    skipped (has changes): $worktree_path"
        fi
    done < <(git worktree list --porcelain | awk '/^worktree /{print $2}' | grep -v "^$ROOT_DIR$")
    git worktree prune
    info "    worktrees removed: $removed"
}

cleanup_merged_branches() {
    info "==> Deleting already merged local branches"
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
    info "    branches removed: $removed"
}

report_unmerged_branches() {
    local unmerged
    unmerged="$(git branch --format='%(refname:short)' --no-merged | grep -v '^main$' || true)"
    if [[ -n "$unmerged" ]]; then
        warn "==> Branches with unmerged work (NOT touched):"
        printf '%s\n' "$unmerged" | sed 's/^/    /'
    fi
}

cleanup_remote_refs() {
    info "==> Pruning stale remote references"
    git fetch --prune --quiet origin 2>/dev/null || warn "    could not run fetch --prune (offline?)"
    local stale
    stale="$(git branch -r | grep -c '\[gone\]' || true)"
    info "    remaining stale remote references: ${stale:-0}"
}

cleanup_git_objects() {
    info "==> Cleaning reflogs and unreachable objects"
    # `git reflog expire --all` also expires refs/stash, and --expire-unreachable
    # prunes stash entries that are not ancestors of the current stash tip.
    # The following `git gc --prune=now` would then delete the stashed commits,
    # losing user work-in-progress.
    #
    # Strategy: preserve the stash reflog by hand, expire everything else, then
    # restore the stash reflog before garbage collection.
    local stash_log stash_backup=""
    stash_log="$(git rev-parse --git-path logs/refs/stash)"
    if [[ -s "$stash_log" ]]; then
        stash_backup="$(mktemp)"
        cp "$stash_log" "$stash_backup"
    fi

    # --all tolerates refs that live only in packed-refs and have no reflog.
    # Expiring per-ref fails with "points nowhere" for those.
    git reflog expire --expire=30.days --expire-unreachable=now --all

    if [[ -n "$stash_backup" ]]; then
        mkdir -p "$(dirname "$stash_log")"
        cp "$stash_backup" "$stash_log"
        rm -f "$stash_backup"
        info "    stash stack preserved"
    fi

    # If refs/stash holds a stash but its reflog is missing or empty, the stash
    # content is still in the object store but `git stash list` cannot show it.
    # Recreate one reflog entry so the stash becomes visible again. The ref is
    # deleted and recreated because `git update-ref` with an unchanged value
    # does not write a reflog entry.
    if git rev-parse --verify --quiet refs/stash >/dev/null && [[ -z "$(git stash list)" ]]; then
        local stash_commit
        stash_commit="$(git rev-parse refs/stash)"
        git update-ref -d refs/stash
        git update-ref --create-reflog -m "clean-git: restore stash reflog" "refs/stash" "$stash_commit"
        info "    refs/stash reflog restored"
    fi

    git gc --prune=now --quiet
    info "    gc completed"
}

main() {
    require_clean_tree
    cleanup_worktrees
    cleanup_merged_branches
    report_unmerged_branches
    cleanup_remote_refs
    cleanup_git_objects
    info ""
    info "Git cleanup completed."
    git worktree list
}

main "$@"
