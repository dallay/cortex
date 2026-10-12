# Cortex — Agent instructions

Cortex includes Rook, an AI provider proxy and router, and Huginn,
a local programming agent. See `README.md` for the current scope.

## Source of truth

- Edit `.agents/AGENTS.md`; the root `AGENTS.md` and the instructions for other
  agents are links managed by `.agents/agentsync.toml`.
  Do not replace those links with standalone files.
- Get the versions from `rust-toolchain.toml`, manifests, and CI.
  Do not duplicate versions or package inventories in this file.
- Consult `Cargo.toml` and `pnpm-workspace.yaml` for workspace limits.
  Keep a single workspace and a root pnpm lockfile; CI rejects copies
  inside the dashboard.
- `justfile` is the canonical runner; `Makefile` is its mirror.
  If you change a recipe, update both.
- MCP configuration and sync destinations are in
  `.agents/agentsync.toml`; do not copy its inventory here.

## Architecture and documentation on demand

Read only the sources related to the change:

- Rook layers, responsibilities, and flow: `docs/architecture.md`.
- HTTP contracts: `docs/api.md`; configuration: `docs/configuration.md`;
  provider behavior: `docs/providers.md`.
- Huginn limits and behavior: `docs/huginn/README.md`.
- Dashboard: `apps/rook/dashboard/README.md`.
- Preparation, tests, and diagnostics: `docs/development.md`.
- Containers and E2E: `dev/README.md`.
- Deployment: `docs/deployment.md`; contributions: `CONTRIBUTING.md`.

If the documentation and code differ, verify the behavior in code,
manifests, and tests before turning the explanation into a rule.

## Work and validation

- Discover the recipes with `just --list`; use `just info` for paths and ports.
- Initial setup: `just setup`; check tools with `just doctor`.
- Focused Rust: `cargo test -p <package>`; an integration target:
  `cargo test -p <package> --test <target> [filter]`.
- Focused dashboard:
  `pnpm --filter @dallay/rook-dashboard exec vitest run [file]`.
- Markdown: `pnpm exec markdownlint-cli2 ".agents/AGENTS.md"` for this source;
  `just ci-lint` for the regular documentation set.
- Use focused tests during iteration. Pre-PR validation is
  `just ci-local`; `just ci-ci` skips E2E and is not equivalent to full validation.
  Check its requirements in `docs/development.md`; do not copy the sequence here.
- E2E requires Docker, dependencies, and browsers installed, plus the
  `just test-e2e-build` image. Consult `dev/README.md` before running it.
- TDD follows the explicit project or session setup. To fix
  behavior errors, add a regression test; do not use tests that lock in SHAs or
  implementation details for documentation changes.
- Report which checks you ran and which failed or were not run.
  Do not present partial validation as full CI.

## Operational limits

- Do not reset data as part of routine testing. If a reset is authorized,
  use `just db-reset`: it removes the SQLite database, WAL, and SHM together.
- Commits use Conventional Commits, without AI attribution.
- Check `.agents/skills/` for specialized procedures and
  `.agents/sdd/` for existing SDD changes. Do not start SDDs solely based on size
  or file count: explicit selection is required.
- Keep this file as an operational contract: invariants and references,
  not copies of the README, directory trees, or tool manuals.
