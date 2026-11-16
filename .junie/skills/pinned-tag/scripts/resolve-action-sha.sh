#!/usr/bin/env bash
# Resolve a tag to the commit SHA that GitHub Actions should pin.
#
# Usage: resolve-action-sha.sh <repo-url> <tag> [--verify]
#
# Without --verify the script performs a single `git ls-remote` and prefers the
# peeled ref (`refs/tags/<tag>^{}`) over the tag object. With --verify it also
# fetches the tag into a temporary repository and confirms the SHA is a commit,
# which rejects tags that point at trees or blobs.
set -euo pipefail

usage() {
    echo "usage: $0 <repo-url> <tag> [--verify]" >&2
    exit 64
}

[ "$#" -ge 2 ] && [ "$#" -le 3 ] || usage
repo_url=$1
tag=$2
verify=0
if [ "$#" -eq 3 ]; then
    [ "$3" = "--verify" ] || usage
    verify=1
fi

[ -n "$repo_url" ] && [ -n "$tag" ] || usage
case "$tag" in
    *[[:space:]]* | *'^{}'*) echo "invalid tag: $tag" >&2; exit 64 ;;
esac

# Exact refs only. A bare pattern would also accept unrelated refixes.
refs=$(git ls-remote "$repo_url" "refs/tags/$tag" "refs/tags/$tag^{}") || {
    echo "cannot read refs from $repo_url" >&2
    exit 1
}

peeled=$(printf '%s\n' "$refs" | awk -v ref="refs/tags/$tag^{}" '$2 == ref { print $1 }')
base=$(printf '%s\n' "$refs" | awk -v ref="refs/tags/$tag" '$2 == ref { print $1 }')

if [ -n "$peeled" ]; then
    sha=$peeled
elif [ -n "$base" ]; then
    sha=$base
else
    echo "tag '$tag' not found in $repo_url" >&2
    exit 1
fi

if ! printf '%s' "$sha" | grep -Eq '^[0-9a-f]{40}$'; then
    echo "resolved value is not a full commit SHA: $sha" >&2
    exit 1
fi

if [ "$verify" -eq 1 ]; then
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/resolve-action-sha.XXXXXX")
    trap 'rm -rf "$tmp"' EXIT
    git init --quiet --bare "$tmp/repo.git"
    # Fetch the tag itself so the server resolves the ref; a raw SHA may not be
    # advertised by every host.
    git -C "$tmp/repo.git" fetch --quiet --depth=1 --no-tags "$repo_url" "refs/tags/$tag" || {
        echo "cannot fetch tag '$tag' from $repo_url" >&2
        exit 1
    }
    fetched_commit=$(git -C "$tmp/repo.git" rev-parse --verify 'FETCH_HEAD^{commit}') || {
        echo "tag '$tag' does not point at a commit" >&2
        exit 1
    }
    object_type=$(git -C "$tmp/repo.git" cat-file -t "$sha" 2>/dev/null || true)
    if [ "$object_type" != "commit" ]; then
        echo "tag '$tag' resolves to a $object_type, not a commit" >&2
        exit 1
    fi
    if [ "$fetched_commit" != "$sha" ]; then
        echo "peeled SHA $sha does not match fetched commit $fetched_commit" >&2
        exit 1
    fi
fi

printf '%s\n' "$sha"
