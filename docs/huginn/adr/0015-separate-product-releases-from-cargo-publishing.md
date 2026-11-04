# ADR-0015: Separate product releases from Cargo publishing

- **Status:** Accepted
- **Date:** 2026-10-11
- **Scope:** Rook GA, later Huginn, and future Cortex products

## Context

Cortex contains products and internal crates with distinct technical boundaries.
Rook can be distributed as a binary via GitHub Releases, npm, and Docker without
making all its internal crates public.

Publishing `rook` on crates.io is not equivalent to the product
release: Cargo requires the transitive dependency closure to be available
in a registry. That would turn technical crates such as `shared-kernel`,
`db-migration`, and `providers-core` into public versioning units, even though they
are not end-user products.

## Decision

We adopt **Option C for Rook GA**:

- GitHub Releases, npm, and Docker Hub/GHCR are mandatory channels.
- `cargo install --git` from the exact tag is the supported Rust channel.
- `cargo install rook` from crates.io does not block the first launch.
- Internal crates keep their current privacy/publishability.
- Huginn is not published until it passes its acceptance gates.
- Publishing on crates.io is kept as an explicit future evolution; it is not
  discarded.

## Alternatives considered

### Publish the transitive closure from the start

It is technically valid and will be the future option if crates.io proves to be a
strategic channel. It requires verifying ownership, possibly renaming generic
crates, and maintaining immutable versions of each dependency.

### Restructure Rook as a self-contained crate

Not adopted. It would change Clean Architecture and technical boundaries to satisfy
a packaging constraint, not a domain need.

## Consequences

### Positive

- Rook reaches GA without prematurely publishing internals.
- The current architecture is preserved.
- Distributed artifacts can be verified by tag and SHA.
- The crates.io decision remains reversible without imposing architectural debt.

### Negative

- `cargo install rook` is not available from crates.io at GA.
- Rust installation requires Git and repository access.
- The later phase will need to compute and publish the actual transitive closure.

## Rules for the crates.io phase

When activated:

1. compute only the transitive closure used by Rook;
2. verify ownership and name collisions;
3. prefer names with a recognizable namespace, such as `cortex-shared-kernel`;
4. publish in topological order;
5. remember that each publication is immutable and each fix requires a new
   version even if the API and dependency graph do not change.

## Separation of configuration, activation, and distribution

Technical preparation and operational activation of Release Please do not require
the product to already be accepted: Release Please can open release PRs, but
the user reviews and decides whether to merge them. Three phases are kept:

1. **Configuration and validation:** prepare the multi-product config/manifest and
   verify its independence.
2. **Release PRs:** Release Please remains active for Rook and Huginn. The user
   keeps control of the merge; merging creates tags and GitHub Releases
   following the normal flow.
3. **Distribution:** publishing binaries or packages on each channel requires the
   product acceptance gate and that channel's preflight.

Activating Release Please does not amount to authorizing distribution. The
current build and publishing jobs remain connected only to Rook. DALLAY-630,
DALLAY-665, and DALLAY-666 are tracked as MVP evidence under DALLAY-634, not
as additional independent gates.

## Non-goals

- Do not turn internal crates into independent products.
- Do not create product tags for every technical crate.
- Do not reorganize the domain solely to enable Cargo.
- Public distribution of Huginn requires DALLAY-634 and that channel's preflight.
