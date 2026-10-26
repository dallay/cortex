---
applyTo: "crates/domain/rook-core/**,crates/application/rook-usecases/**,crates/infrastructure/transport-axum/**,crates/infrastructure/auth-sqlite/**,crates/infrastructure/cache-memory/**,crates/infrastructure/audit-sqlite/**,crates/infrastructure/observability/**,crates/infrastructure/providers-core/**,crates/infrastructure/providers-openai/**,crates/infrastructure/providers-anthropic/**,crates/infrastructure/providers-ollama/**,crates/infrastructure/providers-gemini/**,crates/infrastructure/providers-groq/**,crates/infrastructure/encryption-inmemory/**,crates/infrastructure/provider-sqlite/**,crates/infrastructure/db-migration/**,crates/infrastructure/models-catalog/**,crates/infrastructure/combo-sqlite/**,crates/infrastructure/alias-sqlite/**,apps/rook/**"
---

# Rook — Product instructions

These instructions extend `.agents/AGENTS.md` for Rook. Global invariants remain
mandatory; dashboard instructions additionally apply inside `apps/rook/dashboard`.
All paths below are relative to the repository root.

## Architecture

- Keep models and ports in `crates/domain/rook-core`, orchestration in
  `crates/application/rook-usecases`, and adapters in the explicitly mapped
  infrastructure crates. `apps/rook` is the configuration and DI composition root.
- Dependencies point inward: the domain must not depend on axum, reqwest or
  SQLite; use cases must not depend on transport details. Reuse `ProviderPort`,
  `RouterPort`, `CachePort`, `AuditPort` and established repository ports.
- Keep `shared-kernel`, `test-support` and generic utilities product-neutral.
- Consult `docs/rook/architecture.md` and relevant ADRs in `docs/rook/adr/`.
  For affected contracts, read `docs/rook/api.md`, `docs/rook/configuration.md`
  and `docs/rook/providers.md`; verify claims against implementation and tests.

## Authorization and credentials

- Preserve `AuthTier` (`Public`, `ClientApi`, `Management`) as specified in
  `.agents/sdd/ARCHITECTURE.md`. Management uses the `auth_token` cookie;
  state-changing management requests require CSRF double-submit validation.
  Client API keys travel in `X-API-Key`.
- Remove incoming `X-Authz-*` headers before authorizing; never trust client
  assertions of identity or authorization.
- Preserve the password policy from that contract: Argon2id with at least
  64 MiB memory, 3 iterations, parallelism 4 and a 16-byte random salt.
  Verify actual parameters in `encryption-inmemory`; a documented policy is
  not evidence that an implementation meets it.
- Generate session tokens from 32 cryptographically random bytes and persist
  only the SHA-256 token hash. Never return or log provider credentials.
- Test denied, expired and invalid authentication and cross-connection access,
  alongside successful requests; authorization must precede upstream I/O.

## Routing and performance

- Measure with `cargo test -p transport-axum --test gateway_perf -- --nocapture`;
  consult `docs/perf-baseline.md` for the baseline and limits.
- Preserve circuit-breaker behavior (3 failures, 30-second cooldown) and combo
  deadlines (10 seconds per step, 60 seconds overall), unless intentionally
  changing their contracts with tests and documentation.
- Streaming fallback is allowed only before the first SSE frame.
- Do not assume gateway backpressure exists. Verify current concurrency controls
  before adding parallel work; define a limit and metric for new concurrency.

## Focused validation

- Use deterministic fakes for unit tests of time, providers and persistence;
  avoid real network, disk and clocks in those tests. Isolate adapter integration
  tests at their own boundaries and cover empty, error and denied states.
- Run `cargo test -p <package>` or
  `cargo test -p <package> --test <target> [filter]` for affected crates.
- Read `dev/README.md` before container E2E; read `docs/rook/deployment.md`
  for deployment changes. Never reset persistent data in routine tests.
