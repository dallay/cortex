#!/usr/bin/env bash
# Regression tests for resolve-action-sha.sh using local repositories.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
resolver="$script_dir/resolve-action-sha.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/resolve-action-sha-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

failures=0
pass() { echo "ok - $1"; }
fail() {
    echo "not ok - $1" >&2
    failures=$((failures + 1))
}

git init --quiet "$work/upstream"
git -C "$work/upstream" config user.email test@example.com
git -C "$work/upstream" config user.name Test
# Fixtures must not invoke signing programs inherited from user configuration.
# Command-scoped config also overrides inherited GIT_CONFIG_COUNT settings.
export GIT_CONFIG_COUNT=$((${GIT_CONFIG_COUNT:-0} + 2))
export "GIT_CONFIG_KEY_$((GIT_CONFIG_COUNT - 2))=commit.gpgSign"
export "GIT_CONFIG_VALUE_$((GIT_CONFIG_COUNT - 2))=false"
export "GIT_CONFIG_KEY_$((GIT_CONFIG_COUNT - 1))=tag.gpgSign"
export "GIT_CONFIG_VALUE_$((GIT_CONFIG_COUNT - 1))=false"
printf 'first\n' >"$work/upstream/file.txt"
git -C "$work/upstream" add file.txt
git -C "$work/upstream" commit --quiet -m first
first=$(git -C "$work/upstream" rev-parse HEAD)
printf 'second\n' >>"$work/upstream/file.txt"
git -C "$work/upstream" commit --quiet -am second
second=$(git -C "$work/upstream" rev-parse HEAD)

git -C "$work/upstream" tag v1 "$first"
git -C "$work/upstream" tag -a v1.2 -m "annotated" "$second"
git -C "$work/upstream" tag -a tree-tag -m "points at a tree" "$second^{tree}"

url="$work/upstream"

# Lightweight tags resolve to the commit they point at.
actual=$("$resolver" "$url" v1)
[ "$actual" = "$first" ] && pass "lightweight tag resolves to commit" || fail "lightweight tag: expected $first, got $actual"

# Annotated tags resolve to the peeled commit, not the tag object.
tag_object=$(git -C "$work/upstream" rev-parse 'refs/tags/v1.2')
actual=$("$resolver" "$url" v1.2)
[ "$actual" = "$second" ] && pass "annotated tag resolves to peeled commit" || fail "annotated tag: expected $second, got $actual"
[ "$actual" != "$tag_object" ] && pass "annotated tag does not resolve to tag object" || fail "annotated tag returned tag object"

# Exact matching: v1 must not be satisfied by v1.2, and a prefix is not a tag.
actual=$("$resolver" "$url" v1)
[ "$actual" = "$first" ] && pass "tag matching is exact" || fail "exact tag matching: expected $first, got $actual"
if "$resolver" "$url" v >/dev/null 2>&1; then
    fail "tag prefix should not resolve"
else
    pass "tag prefix does not resolve"
fi

# Missing tags fail closed with a non-zero status.
if "$resolver" "$url" missing >/dev/null 2>&1; then
    fail "missing tag should fail"
else
    pass "missing tag fails"
fi

# --verify accepts a normal lightweight tag.
if actual=$("$resolver" "$url" v1 --verify); then
    [ "$actual" = "$first" ] && pass "--verify accepts a commit" || fail "--verify returned $actual"
else
    fail "--verify rejected a valid commit"
fi

# --verify rejects tags that point at a tree.
if "$resolver" "$url" tree-tag --verify >/dev/null 2>&1; then
    fail "--verify should reject a tree tag"
else
    pass "--verify rejects a tree tag"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures test(s) failed" >&2
    exit 1
fi
echo "all resolve-action-sha tests passed"
