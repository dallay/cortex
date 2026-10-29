---
name: pinned-tag
description: Use when fixing "Unpinned tag for a non-immutable Action" warnings or when pinning dependencies to specific git versions.
allowed-tools: Read, Edit, Write, Glob, Grep, Bash
license: Apache-2.0
metadata:
  author: yuniel
  version: "2.0"
---

# Pinned Tag Management

Resolve a mutable tag to the immutable commit SHA that GitHub Actions should
execute, and apply the result to a workflow without breaking the build.

## When to use

- A security scan reports "Unpinned tag for a non-immutable Action".
- You are adding a third-party Action and must pin it from the start.
- You need to refresh an existing pin after an upstream release.

## Non-negotiables

- **Pin Actions to the full 40-character commit SHA**, never to a branch or a
  tag. Tags and branches are mutable references controlled by the upstream
  repository.
- **Keep the tag in a trailing comment** (`# v4.2.2`) so reviewers can tell what
  the pin means. Dependabot and Renovate use that comment to update the pin.
- **Resolve the exact ref.** Do not filter with a loose pattern and pick the
  last line: `sort` orders the whole line, whose first field is a random SHA, so
  it does not deterministically prefer the peeled `^{}` ref.
- **Never invent a SHA.** If the ref cannot be resolved, stop and report it.

## Resolve a SHA

Use the script in this skill instead of ad-hoc pipelines:

```bash
# Read-only resolution (one network round trip)
.agents/skills/pinned-tag/scripts/resolve-action-sha.sh \
  https://github.com/actions/checkout.git v4
# -> 11bd71901bbe5b1630ceea73d27597364c9af683

# Verify that the SHA is a commit, not a tag object, tree or blob
.agents/skills/pinned-tag/scripts/resolve-action-sha.sh \
  https://github.com/actions/checkout.git v4 --verify
```

The script matches `refs/tags/<tag>` and `refs/tags/<tag>^{}` exactly, prefers
the peeled ref for annotated tags, rejects values that are not 40 hex
characters, and with `--verify` shallow-fetches the tag into a temporary
repository to confirm the object type and that the fetched commit equals the
resolved SHA.

If you must do it by hand, keep the matching exact and the precedence explicit:

```bash
refs=$(git ls-remote --tags https://github.com/owner/repo.git \
  "refs/tags/v1" "refs/tags/v1^{}")
sha=$(printf '%s\n' "$refs" | awk -v ref='refs/tags/v1^{}' '$2 == ref { print $1 }')
[ -n "$sha" ] || sha=$(printf '%s\n' "$refs" | awk -v ref='refs/tags/v1' '$2 == ref { print $1 }')
printf '%s\n' "$sha" | grep -Eq '^[0-9a-f]{40}$'
```

Avoid `rg` in these snippets: it is not guaranteed to be installed, while
`awk` and `grep` are available in the supported environments.

## Apply the pin

1. Find `uses: owner/repo@ref` in the workflow.
2. Resolve the SHA for that exact tag.
3. Replace the ref and append the tag as a comment:

```yaml
# Before
- uses: actions/checkout@v4
# After
- uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4
```

4. Re-run the workflow. A pin that no longer matches the expected inputs is a
   real failure; do not "fix" it by returning to the tag.

## Limitations

- Private repositories need credentials (`GIT_ASKPASS`, an SSH agent or a
  token-backed URL). Do not paste tokens into workflow files or commands.
- Local tags can differ from the remote. Always resolve against the remote URL
  that the workflow actually uses.
- `git ls-remote` shows refs, not object types; use `--verify` when the tag was
  not created by a release process you trust.
- A tag can be moved after pinning. Pinning limits the blast radius; it does not
  make upstream code trustworthy.

## Commit policy

- Do not commit without explicit permission.
- Suggested message: `sec(ci): pin <action-name> to commit SHA for immutability`

## Resources

- [GitHub: security hardening for GitHub Actions](https://docs.github.com/en/actions/security-guides/security-hardening-for-github-actions#using-third-party-actions)
- [GitHub: managing custom actions](https://docs.github.com/en/actions/how-tos/create-and-publish-actions/manage-custom-actions)
- `scripts/resolve-action-sha.sh` and `scripts/resolve-action-sha.test.sh`
