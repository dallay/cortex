# Cortex — Agent Instructions

Cortex is a production-oriented AI infrastructure monorepo. It includes Rook, an AI provider proxy and router, and Huginn, a local programming agent. See `README.md` for the current product scope.

This document defines the engineering principles, non-negotiable invariants, and operational boundaries shared by every Cortex product and interface.

Product-specific instructions extend this contract through nested agent contexts. They must not weaken its global invariants.

## Engineering priorities

Correctness, data integrity, reliability, and verifiable behavior are mandatory baseline requirements.

When making engineering trade-offs, apply these priorities in order:

1. **Security first.** Protect credentials, user data, authorization boundaries, execution permissions, and system integrity. Never trade away security for performance, convenience, or visual polish.
2. **Performance second.** Design for predictable latency, efficient resource usage, concurrency safety, and scalability. Prefer measured improvements over speculative optimization.
3. **User experience third.** Deliver intuitive, accessible, consistent, and responsive experiences across every applicable interface: web, TUI, CLI, desktop, and future surfaces.

These priorities guide trade-offs; they do not excuse broken functionality or incomplete implementations.

When priorities conflict, preserve the higher-priority invariant and explain the trade-off.

## Architecture and design principles

- **Hexagonal architecture (Ports and Adapters)** is the default architectural model. Keep business rules and application behavior independent of frameworks, transports, databases, providers, and presentation technologies.
- Dependencies point inward. Infrastructure and presentation implement or consume application-facing contracts, never the reverse.
- Domain models own business invariants. Application services orchestrate use cases; adapters translate between external systems and internal contracts.
- Use **SOLID** pragmatically: cohesive responsibilities, explicit contracts, substitutable implementations, focused interfaces, and dependency inversion.
- Apply **DRY, KISS, and YAGNI**. Eliminate meaningful duplication, favor simple designs, and do not introduce abstractions for hypothetical requirements.
- Use established **design patterns** when they solve a concrete problem. Prefer composition over inheritance and explicit dependencies over hidden coupling.
- Reuse existing boundaries and conventions before introducing new layers, traits, interfaces, dependencies, or patterns.
- Keep shared abstractions product-neutral. Do not leak Rook-specific or Huginn-specific behavior into shared components.
- Preserve public contracts and backward compatibility unless an intentional breaking change is explicitly authorized.
- Favor maintainability, testability, and clear ownership over cleverness.

Hexagonal architecture is a dependency and responsibility model, not a requirement to create an interface for every type or to reproduce identical layers in every module.

## Security by design

- Treat all external input as untrusted, including HTTP requests, provider responses, model output, tool results, files, and third-party integrations.
- Validate inputs at trust boundaries and enforce authorization before accessing protected capabilities or resources.
- Apply least privilege, secure defaults, explicit permission boundaries, and defense in depth.
- Never hardcode, expose, or log credentials, tokens, secrets, or sensitive payloads.
- Preserve workspace `unsafe_code = "deny"` and clippy with `-D warnings`. Do not weaken them to make a change compile.
- Preserve the blocking CI security gates: gitleaks (`.gitleaks.toml`), semgrep ERROR findings for Rust, Dockerfiles and GitHub Actions, trivy, and cargo audit. `just ci-audit` is advisory and does not replace those gates.
- Fail securely on authorization and integrity violations. Do not silently bypass security controls to maintain availability.
- Protect against injection, path traversal, unauthorized execution, sensitive data disclosure, and resource exhaustion where applicable.
- Evaluate the security impact of new dependencies, external integrations, and changes to execution permissions.

## Performance and resource efficiency

- Consider algorithmic complexity, allocations, memory usage, I/O, concurrency, and backpressure when designing hot paths.
- Avoid unnecessary blocking, unbounded work, redundant network calls, and uncontrolled resource consumption.
- Establish a baseline before making performance claims. Use benchmarks, profiling, tracing, or representative measurements when optimization is relevant.
- Do not sacrifice correctness, security, or maintainability for unverified micro-optimizations.
- Keep performance-sensitive behavior observable and test important resource limits.

## User experience and interface quality

- Treat UX/UI as an engineering responsibility, not merely visual styling.
- Design for clarity, predictable behavior, discoverability, consistency, and minimal cognitive load.
- Prefer contextual actions, appropriate navigation, dialogs, and menus over cluttered or confusing interaction flows.
- Provide meaningful loading, empty, success, error, and recovery states where relevant.
- Preserve accessibility, keyboard navigation, focus management, and understandable feedback across applicable interfaces.
- Respect the interaction conventions and constraints of each surface, including web, TUI, CLI, and desktop.
- Validate user-facing changes through appropriate interaction, accessibility, and visual checks. Do not consider an interface complete merely because it renders.

Product-specific design systems, component conventions, and interaction contracts belong in nested instructions or dedicated documentation.

## Test-driven development and quality

- **TDD is the default workflow for changes to executable behavior:** Red → Green → Refactor.
- Start with a focused failing test expressing the expected behavior. Implement the smallest correct solution, then refactor while keeping tests green.
- For bugs, reproduce the failure with a regression test before fixing it whenever feasible.
- Test observable behavior, domain invariants, boundaries, and failure modes rather than incidental implementation details.
- Use unit, integration, contract, and end-to-end tests at the appropriate boundaries. Prefer deterministic tests.
- For security-sensitive changes, test unauthorized and adversarial scenarios. For performance-sensitive changes, add representative measurements when warranted.
- For interface changes, validate user interactions and relevant UI states, not only rendered output.
- Documentation-only, exploratory, and other non-behavioral changes do not require artificial failing tests. Explain justified deviations from the default workflow.
- Never weaken, delete, or bypass meaningful tests merely to make a change pass.

## Source of truth

- Edit `.agents/AGENTS.md`; root-level instructions and agent-specific destinations are managed through `.agents/agentsync.toml`. Do not replace managed symlinks with standalone copies.
- Keep global principles here and product-specific rules in `.agents/contexts/`. See `.agents/contexts/README.md` for scope and synchronization checks. After editing canonical instructions or mappings, run `pnpm agents:sync`; avoid duplicated or conflicting instructions.
- Derive versions from `rust-toolchain.toml`, manifests, and CI. Do not duplicate versions or package inventories.
- Consult `Cargo.toml`, root `package.json`, and CI for workspace boundaries. Preserve the single Cargo workspace and root `pnpm-lock.yaml`; CI rejects `pnpm-lock.yaml` or `pnpm-workspace.yaml` inside the dashboard. Use pnpm exclusively.
- `justfile` is the canonical task runner. Do not assume a `Makefile` exists; none is tracked in this repository.
- MCP definitions and synchronization destinations belong in `.agents/agentsync.toml`, not this document.

## Documentation on demand

Read only the references relevant to the affected scope:

- Rook architecture: `docs/rook/architecture.md`.
- Rook APIs, configuration, and providers: `docs/rook/api.md`, `docs/rook/configuration.md`, and `docs/rook/providers.md`.
- Huginn overview and architecture: `docs/huginn/README.md` and `docs/huginn/architecture.md`.
- Rook dashboard: `apps/rook/dashboard/README.md`.
- Development workflows and diagnostics: `docs/development.md`.
- Containers and E2E: `dev/README.md`.
- Deployment: `docs/rook/deployment.md`.
- Contributions: `CONTRIBUTING.md`.

If documentation and implementation disagree, verify the behavior against code, manifests, and tests before establishing or updating a rule.

## Development workflow and validation

- Inspect applicable nested instructions, existing conventions, and relevant tests before changing code.
- Check `.agents/skills/` for specialized procedures and `.agents/sdd/` for existing SDD changes. Start new SDD workflows only when explicitly selected.
- Discover recipes using `just --list`. Consult `docs/development.md` and `dev/README.md` for environment details and ports.
- Use `just setup` for initial setup and `just doctor` for read-only tool diagnostics.
- Run focused checks during iteration:
  - Rust: `cargo test -p <package>` and relevant integration targets.
  - Dashboard: `pnpm --filter @dallay/rook-dashboard exec vitest run [file]`.
  - Markdown: `pnpm exec markdownlint-cli2 ".agents/AGENTS.md" ".agents/contexts/*.md"` for canonical instructions.
- Run `just ci-local` for full pre-PR validation when its documented requirements are available. `just ci-ci` skips E2E and is not equivalent.
- Check `docs/development.md` and `dev/README.md` for validation prerequisites. E2E requires its documented dependencies and the `just test-e2e-build` image.
- Keep changes focused. Avoid unrelated refactoring, speculative infrastructure, and unnecessary dependencies.
- Update relevant documentation and tests whenever observable behavior or contracts change.
- Report the checks executed, their outcomes, and any checks omitted or blocked. Never present partial verification as full CI.

## Operational boundaries

- Never reset persistent data as part of routine testing. An authorized reset must use `just db-reset` to remove the SQLite database, WAL, and SHM consistently.
- Do not execute destructive operations, expose secrets, or change production resources without the appropriate authorization.
- Keep durable change artifacts in `.agents/sdd/changes/` and RPI tasks in `.agents/rpi/plan/tasks/`.
- Use Conventional Commits without AI attribution.
- Keep this file an operational contract: durable principles, critical invariants, and references—not duplicated README content, directory inventories, or tool manuals.
