# ADR-0015: Separate Product Releases from Cargo Publishing

- **Status:** Accepted
- **Date:** 2026-10-11
- **Scope:** Rook GA, subsequent Huginn, and future Cortex products

## Context

Cortex contains products and internal crates with distinct technical boundaries.
Rook can be distributed as a binary via GitHub Releases, npm, and Docker without
making all its internal crates public.

Publishing `rook` on crates.io is not equivalent to product release:
Cargo requires the transitive closure of dependencies to be available in a
registry. That would force technical crates such as `shared-kernel`,
`db-migration`, and `providers-core` into public versioned units, even though
they are not end-user products.

## Decision

We adopt **Option C for Rook GA**:

- GitHub Releases, npm, and Docker Hub/GHCR are mandatory channels.
- `cargo install --git` from the exact tag is the supported Rust channel.
- `cargo install rook` from crates.io does not block the initial release.
- Internal crates maintain their current private/publishable status.
- Huginn is not published until passing its acceptance gates.
- crates.io publishing is preserved as an explicit planned evolution, not discarded.

## Alternatives Considered

### Publishing transitive closure from the start

Technically valid and will be a future option if crates.io proves to be a
strategic channel. Requires verifying crate ownership, potentially renaming generic
crates, and maintaining immutable versions for every dependency.

### Restructuring Rook to be a self-contained crate

Not adopted. Would compromise Clean Architecture and technical boundaries to satisfy
packaging constraints rather than domain needs.

## Consequences

### Positive

- Rook achieves GA without prematurely publishing internals.
- Preserves existing architecture.
- Distributed artifacts can be verified by tag and SHA.
- The crates.io decision remains reversible without imposing architectural debt.

### Negative

- `cargo install rook` is not available from crates.io at GA.
- Rust installation requires Git and repository access.
- A future phase will need to calculate and publish the actual transitive closure.

## Rules for crates.io Phase

When activated:

1. Calculate only the transitive closure used by Rook;
2. Verify ownership and name collisions;
3. Prefer names with recognizable namespaces, such as `cortex-shared-kernel`;
4. Publish in topological order;
5. Remember that every publication is immutable and every fix requires a new
   version even if API and dependency graph remain unchanged.

## Separation of Configuration, Activation, and Distribution

Technical preparation and operational activation of Release Please do not require
prior product acceptance: Release Please can open release PRs, but the operator
reviews and decides whether to merge them. Three phases are maintained:

1. **Configuration and validation:** prepare multi-product config/manifest and
   verify independence.
2. **Release PRs:** Release Please is activated for Rook and Huginn. The operator
   retains merge control; merging generates tags and GitHub Releases according
   to standard workflows.
3. **Distribution:** publishing binaries or packages across each channel requires
   product acceptance gates and channel preflights.

Release Please activation is not equivalent to authorized distribution. Current
build and publication jobs remain connected only to Rook. DALLAY-630,
DALLAY-665, and DALLAY-666 are tracked as MVP evidence in DALLAY-634, not as
additional independent gates.

## Non-Goals

- Do not convert internal crates into standalone products.
- Do not create product tags for every technical crate.
- Do not reorganize domain boundaries solely to enable Cargo publishing.
- Public Huginn distribution requires DALLAY-634 and channel preflight.
