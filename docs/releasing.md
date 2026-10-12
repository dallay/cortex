# Cortex Release and Distribution Contract

This document defines how Cortex products are released. Keep three concepts
separate:

1. **Products:** Rook and Huginn.
2. **Architecture:** internal crates organized by responsibility and boundaries.
3. **Distribution:** the channels through which each product is installed.

These do not need to share a lifecycle.

## Product identity

Cortex is the repository and ecosystem. Its current products are:

| Product | Binary | Primary code | Tag | Public status |
| --- | --- | --- | --- | --- |
| Rook | `rook` | `apps/rook` | `rook@vX.Y.Z` | Active pipeline; public distribution requires preflight |
| Huginn | `huginn` | `apps/huginn` | `huginn@vX.Y.Z` | Release Please active; public distribution awaits MVP acceptance and channel preflight |

There is no `cortex@vX.Y.Z` product release. Historical unprefixed Rook tags
are retained and never rewritten. New releases use `rook@vX.Y.Z`.

## Release Please

The repository uses manifest mode:

- `.release-please-manifest.json` and `release-please-config.json` track Rook
  and Huginn as independently versioned products.
- Activating Huginn in Release Please allows it to generate release PRs and
  product tags. A human controls whether those PRs are reviewed and merged.
- Build, GitHub asset, and distribution jobs consume only Rook's outputs.
  Tracking Huginn in Release Please does not add it to those channels.
- `separate-pull-requests: true` keeps product release PRs separate.
- `always-update: true` synchronizes product metadata.
- `include-component-in-tag: true` and `tag-separator: "@"` make product tags
  explicit.
- The `cargo-workspace` plugin can update actual Rust dependencies. However,
  repository co-location alone must not release an unrelated product.

The production workflow consumes only path-scoped outputs:

- `apps/rook--release_created`
- `apps/rook--version`
- `apps/rook--tag_name`
- `apps/rook--sha`

The workflow also reads Huginn's path-scoped outputs
(`apps/huginn--release_created`, `apps/huginn--version`,
`apps/huginn--tag_name`, and `apps/huginn--sha`), but no distribution jobs are
connected to them. Root-level outputs are prohibited.

Every distribution job checks out the exact product tag and verifies that
`HEAD` matches the SHA provided by Release Please. Builds must use that immutable
source revision. Retries for the same tag must either verify and reuse matching
artifacts or fail; they must not silently replace artifacts from another SHA.

Before enabling any public distribution channel, record evidence for:

1. Channel ownership and credentials.
2. Product-specific supported targets and smoke tests.
3. Clean installation, checksum validation, and `<binary> --version`.
4. Artifact-to-tag-to-SHA traceability.
5. Explicit go/no-go authorization for that channel.

## Distribution channels

Each channel has its own gate. `RELEASE_PUBLICATION_ENABLED` protects Rook's
npm and Docker publication jobs. The human decision to merge a Rook release PR
is the go/no-go for creating its GitHub Release; the required channel checks
must be complete before that merge. GitHub Release asset upload, npm, Docker
Hub, GHCR, and crates.io are distinct distribution actions and must not be
inferred from one another.

### GitHub Releases — required

A Rook release contains its archives and separate `.sha256` checksum sidecars.
Asset names include the product and target, for example
`rook-v1.2.3-x86_64-unknown-linux-gnu.tar.gz`. Each sidecar contains the SHA-256
checksum for its corresponding `.tar.gz` or `.zip` archive; use that sidecar to
verify the downloaded archive. The archives do not contain `SHA256SUMS`. A
release summary records the version, tag, SHA, and distribution job results.

### Cargo — `cargo install --git`

Rook GA supports installation directly from the Git repository. This does not
publish internal crates to crates.io. A crates.io release is deferred until
there is a separate product and ownership decision for every crate intended for
publication.

### npm — required for Rook

The package `@dallay/rook` is a platform-independent wrapper. It declares an
optional dependency on exactly one platform package, whose lifecycle scripts
are disabled. The wrapper resolves the platform package's binary and reports
clear errors if it is unavailable. Platform packages contain the target binary
at `bin/rook` and declare no install scripts.

Every npm package uses the product version. Publication requires an authorized
release, verified provenance and ownership, matching platform artifacts, and a
successful package validation. Retries must not overwrite an already published
version with different contents.

### Docker — required for Rook

The Rook image uses a prepared Linux/amd64 binary at
`docker-context/rook-amd64`. It runs as a non-root user, exposes port 8080,
and uses port 8080 for its health check. Multi-architecture publication
remains disabled until every target has product-specific build and smoke-test
evidence.

### Huginn — future distribution

Huginn distribution requires DALLAY-634 MVP acceptance, stable public
`HUGINN_*` contracts, Ratatui/PTY evidence, and the preflight for each channel.
Huginn must define its own target matrix, binary naming, checksums, npm package,
and container contract. It must not inherit Rook's packaging assumptions.

## Release inputs and validation

`workflow_dispatch` with `dry_run: true` validates the workflow YAML and Release
Please JSON syntax only. It does not run Release Please, create release PRs or
tags, create GitHub Releases, publish registry packages, or push images. Normal
workflow runs consume the active `release-please-config.json` and
`.release-please-manifest.json`.

Run `actionlint .github/workflows/release.yml` and the YAML/JSON syntax checks
when changing workflow or Release Please configuration. Publication jobs must
remain conditioned on explicit release outputs and the channel authorization
gate. A green syntax check is not proof of Release Please path-scoping behavior;
verify release calculations with the actual Release Please tool before claiming
that internal crate changes trigger the intended product.

## Retries and partial releases

A failed platform build blocks aggregate GitHub assets and the npm wrapper that
depends on all platform packages. Docker publication is independent and may
succeed while npm fails. A retry must inspect registry state first, skip exact
published versions, and never overwrite a conflicting version. Publish missing
channels without rebuilding or mutating an artifact associated with an existing
tag.

## Ownership and setup

Each product must own:

- one manifest entry and one package configuration;
- one changelog, component tag, and release PR;
- path-scoped Release Please outputs;
- product-specific build and distribution jobs;
- channel ownership, credentials, preflight evidence, and retry rules.

A future product such as Muninn must not create a global Cortex version or
change existing component identities. Add it as a separate manifest-mode
component after defining its ownership, path scope, tag, artifacts, distribution
channels, and gates.
