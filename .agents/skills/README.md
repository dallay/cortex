---
specs: https://agentskills.io/specification
---
# Agent Skills — Cortex

Catalog of engineering and product skills for the monorepo. Each skill lives in
its own directory with a `SKILL.md`; the legacy Rust guides are standalone
Documents under `languages-typing/rust/`. All skills should follow the official specs [Specification - Agent Skills](https://agentskills.io/specification)

Structural validation is required in CI:

```bash
node scripts/validate-skills.mjs
node --test scripts/validate-skills.test.mjs
```

The validator checks frontmatter structure (including space-separated
`allowed-tools`), `name`/directory identity, duplicates, description length, size,
and local links. **It does not certify** technical claims, versions, or commands:
verify those against manifests, code, and tests.

## Discovery: Two Consumers

`.agents/skills/` has two distinct uses, and they do not share the same mechanism:

| Consumer                                             | What it uses                                  | Scope                 |
|------------------------------------------------------|-----------------------------------------------|-----------------------|
| Engineering agents (OpenCode, Claude Code, Codex)    | Full catalog, synchronized with AgentSync     | Any depth             |
| Huginn (`FilesystemSkillCatalog`)                    | Direct children of `.agents/skills/` only     | One level             |

Consequences:

- `pinned-tag/SKILL.md` is **discoverable** by Huginn.
- `tui/ratatui-tui/SKILL.md` is an engineering skill and is **not** discoverable
  by the current catalog. This is intentional; changing the discovery contract
  requires a product decision and tests for nesting depth, duplicates, and
  symbolic links.
- Huginn validates up to 65,536 bytes per file, but `load_skill` has a
  16 KiB budget for the JSON response. A skill can be **valid** but not
  **loadable**; check its size before adding extensive content.

## Product Skills

### [pinned-tag](pinned-tag/SKILL.md)

Pins GitHub Actions to immutable SHAs and resolves the correct reference (`v4`, `v4.2.2`, annotated tags) with `scripts/resolve-action-sha.sh`.

### [tui/ratatui-tui](tui/ratatui-tui/SKILL.md)

Operational contracts for the Huginn TUI: single terminal owner, inline Ratatui
rendering, layout, Unicode, focus/overlays, lifecycle, and PTY tests.

## Frontend

Base path: `frontend-platform/`.

| Skill                                                                 | Use case                                                          |
|-----------------------------------------------------------------------|-------------------------------------------------------------------|
| [vue](frontend-platform/vue/SKILL.md)                                 | Components, composables, Pinia stores, dashboard forms             |
| [shadcn-vue](frontend-platform/shadcn-vue/SKILL.md)                   | shadcn-vue components and registry                                 |
| [best-practices](frontend-platform/best-practices/SKILL.md)           | Web security, compatibility, quality, and performance              |
| [accessibility](frontend-platform/accessibility/SKILL.md)             | WCAG, keyboard navigation, screen readers                          |
| [core-web-vitals](frontend-platform/core-web-vitals/SKILL.md)         | LCP, CLS, and INP                                                  |
| [performance](frontend-platform/performance/SKILL.md)                 | Load-time and runtime optimization                                 |
| [seo](frontend-platform/seo/SKILL.md)                                 | Metadata, structured data, sitemaps                                |
| [astro](frontend-platform/astro/SKILL.md)                             | Astro, SSG, and islands                                            |
| [frontend-design](frontend-platform/frontend-design/SKILL.md)         | Component and page design                                          |
| [animate-text](frontend-platform/animate-text/SKILL.md)               | Named text effects                                                 |
| [nothing-design](frontend-platform/nothing-design/SKILL.md)           | Nothing-inspired design                                            |
| [modern-web-guidance](frontend-platform/modern-web-guidance/SKILL.md) | Current web APIs and patterns                                      |
| [chrome-extensions](frontend-platform/chrome-extensions/SKILL.md)     | Manifest V3 extensions and Chrome Web Store                        |

## Testing

Base path: `testing/`.

| Skill                                                                   | Use case                                       |
|-------------------------------------------------------------------------|------------------------------------------------|
| [vitest](testing/vitest/SKILL.md)                                       | Unit and component tests; Vitest 5.0.5         |
| [playwright](testing/playwright/SKILL.md)                               | Dashboard E2E; `apps/rook/dashboard/e2e`       |
| [playwright-best-practices](testing/playwright-best-practices/SKILL.md) | Advanced Playwright patterns                   |

## Languages and Typing

| Skill                                                               | Use case                                 |
|---------------------------------------------------------------------|------------------------------------------|
| [typescript](languages-typing/typescript/SKILL.md)                  | Types, interfaces, and generics          |
| [rust-anti-patterns](languages-typing/rust/rust-anti-patterns.md)   | Common anti-patterns                     |
| [rust-api-design](languages-typing/rust/rust-api-design.md)         | Public API design                        |
| [rust-async-patterns](languages-typing/rust/rust-async-patterns.md) | Tokio and concurrency                    |
| [rust-error-handling](languages-typing/rust/rust-error-handling.md) | `CortexError`, `thiserror`, propagation  |
| [rust-linting](languages-typing/rust/rust-linting.md)               | Lints and enforcement                    |
| [rust-documentation](languages-typing/rust/rust-documentation.md)   | Public docs, examples, and safety        |
| [rust-naming](languages-typing/rust/rust-naming.md)                 | Naming conventions                       |
| [rust-memory](languages-typing/rust/rust-memory.md)                 | Allocation, cloning, and zero-copy       |
| [rust-testing](languages-typing/rust/rust-testing.md)               | Tests, doctests, and benchmarks          |
| [rust-tooling](languages-typing/rust/rust-tooling.md)               | `just`, coverage, auditing, and debugging |

## Monorepo Architecture

```text
crates/
├── domain/
│   ├── shared-kernel/     # CortexError, IDs, no infrastructure dependencies
│   └── rook-core/         # Ports and domain model
├── application/
│   └── rook-usecases/     # Use cases, routing, fallback
├── infrastructure/
│   ├── providers-*/       # Provider adapters
│   ├── transport-axum/    # HTTP adapter
│   └── …                  # Cache, auditing, persistence
├── huginn/                # Local agent: domain, application, runtime, presentation
└── test-support/          # Shared test utilities
```

The skills follow this structure and its dependency direction.

## Application Rules

1. **Domain (`shared-kernel`, `rook-core`)** — do not use `anyhow`.
    - Use `CortexError` and domain-specific errors.
    - Use `thiserror` for error definitions.
2. **Application (`rook-usecases`)** — use typed errors.
    - Propagate provider errors.
    - Do not catch and hide failures.
3. **Infrastructure (`providers-*`, `transport-axum`)** — adapt errors.
    - Convert external errors to `CortexError`.
    - Document errors from external APIs.
4. **Binaries (`apps/rook`)** — may use `anyhow` at the boundary.
    - Add context, logging, and HTTP responses.

## Sources

- [rust-skills](https://github.com/leonardomso/rust-skills) — original collection
- [Cortex architecture](../../docs/rook/architecture.md)
- [Development guide](../../docs/development.md)
