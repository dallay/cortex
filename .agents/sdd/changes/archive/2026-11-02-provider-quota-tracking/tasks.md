# Tasks: Provider Quota Tracking

## Review Workload Forecast

| Field                   | Value                                                       |
|-------------------------|-------------------------------------------------------------|
| Estimated changed lines | 750–950                                                     |
| 400-line budget risk    | High                                                        |
| Chained PRs recommended | No (delivery requested in one ordinary PR)                  |
| Suggested split         | One PR: cost → backend contract → dashboard → verification  |
| Delivery strategy       | single-pr                                                   |
| Chain strategy          | single-pr                                                   |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: single-pr
400-line budget risk: High

The user authorized consolidating the whole scope into a single PR even though the stack complicates delivery; this approves the size exception for this delivery (estimated 750–950 lines). Rationale: avoid coordination complexity across three layers and deliver the integrated end-to-end flow. Base `main`, issue/Linear #132. The full technical scope is kept; no stacked branches/layers and no GitHub Stack metadata are created. Cortex usage does not represent the account total; do not invent limits.

### Suggested Work Units

| Unit | Goal                                                                                      | Likely PR | Notes                                                                                                                           |
|------|-------------------------------------------------------------------------------------------|-----------|---------------------------------------------------------------------------------------------------------------------------------|
| 1    | Complete costs, truthful backend summary and localized dashboard with trends and refresh.  | Single PR | Base=`main`; issue/Linear=#132. Start: `cost_usd` aggregate; end: phase technical criteria and verifications completed. |

## Project Test Mode and Runners

- `.agents/sdd/config.yaml`: `strict_tdd: false`; test-first only for pure logic with an adequate seam (threshold classification).
- Rust: `cargo test --workspace --all-features`; dashboard: `cd apps/rook/dashboard && pnpm exec vitest run`. No SDD quality runner is configured.

## Phase 1: Cost Completeness

- [x] 1.1 Extend `CostBreakdown` in `crates/domain/rook-core/src/model.rs` with known/unknown counts, preserving the total semantics.
- [x] 1.2 Update `SqliteUsageRepository::cost_breakdown` in `crates/infrastructure/audit-sqlite/src/lib.rs`: `SUM(cost_usd)`, `COUNT(cost_usd)` and a count matching the filters. Test explicit zero, with/without price, mixed, empty window and connection/date filters.
- [x] 1.3 Adjust `UsageRecorderPort` mocks only if necessary; run focused tests of audit-sqlite and dependent crates.

## Phase 2: Truthful Backend Summary

- [x] 2.1 Extract a pure per-connection classifier in `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs`; test-first for warning/error limits and unavailable inputs.
- [x] 2.2 Replace the legacy DTO in `crates/infrastructure/transport-axum/src/provider_dto.rs`: coverage, availability, cost completeness, `accountQuota`, grouped thresholds, rate-limit observation, alert level, periods and trends; preserve the route and camelCase.
- [x] 2.3 In `provider_quota.rs`, build rows for the seven provider types from connections; expose Cortex-only coverage, configuration, thresholds and correct availability for local/Cloud Ollama.
- [x] 2.4 Map all-time/24 h/7-day data and seven UTC buckets to explicit states; isolate recorder failures per measure/window and keep unknown cost as null.
- [x] 2.5 Add route tests in `crates/infrastructure/transport-axum/tests/` with a configurable fake recorder and `ManageConnections`: JSON response/provenance coverage, absent recorder, failure of one summary query while keeping successful windows, partial cost represented as `null` and aggregation per provider/connection. Verified with `cargo test -p transport-axum --test provider_quota_routes` (4 tests).

## Phase 3: Dashboard Usage and Refresh

- [x] 3.1 Reflect the enums, nullable costs and contract in `apps/rook/dashboard/src/lib/api.ts`; keep the `getProvidersQuota()` URL.
- [x] 3.2 Extend `useProvidersQuota.ts` and `useProvidersQuota.spec.ts`: manual/60 s refresh, overlaps, last valid data, stale/recovery, initial failure and cleanup on unmount.
- [x] 3.3 Update `ProvidersQuotaView.vue` and its spec: coverage/provenance, unavailable vs no data, local alerts, account quotas, unknown costs/providers and stale refresh.
- [x] 3.4 Show 7-day trends (requests/tokens/known estimated cost) with existing primitives; add an accessible equivalent and empty/unavailable cases.
- [x] 3.5 Update `apps/rook/dashboard/src/locales/en.json`, `locales/es.json` and `src/__tests__/providersI18n.spec.ts`; validate Vitest and typecheck. Remove `ChartContainer` to avoid the unnecessary `@unovis/vue` dependency.

## Phase 4: Integrated Verification

- [x] 4.1 Contrast both delta specs with focused suites and run `just ci-local`. Evidence: both delta specs checked with `cargo test -p transport-axum --test provider_quota_routes` (13 tests), `cargo test -p rook-usecases`, `cargo test -p rook-core` (61 tests) and the dashboard suite (44 files, 386 tests). `just ci-local` re-run after freeing space: all gates passed (install, CI contract, doctor, issue-labeler, dashboard build, Markdown lint, fmt, Clippy, Huginn architecture + cargo check, Rust tests 457s, frontend typecheck/Biome/Vitest, Rust doc, cargo audit, Trivy, Gitleaks). Semgrep failed because of a local toolchain incompatibility (`protobuf` from `semgrep==1.126.0` under Python 3.14 chosen by `uv`) and passed when re-run with `--python 3.12`: 19 rules, 252 targets, 11 findings with exact exceptions, non-suppressed check `true`. Additional gates: `just ci-coverage-frontend` passed, `just ci-coverage-rust` passed with `--fail-under-lines 87`,
  `just ci-e2e` passed with 90 tests. SonarCloud and the push-only build are outside the local set by design and are not reported as passed.
- [x] 4.2 Confirmed by `git diff --name-only` and reading `routes.rs`: `V2__usage_history.sql`, adapters, routing, DI, admin authorization and the `/api/providers/quota` URL do not change. The handler/DTO and dashboard change deliberately to satisfy the contract; the route test invokes the handler through a test Router and does not revalidate the existing admin middleware.

## Phase 5: Upstream Capture (U1–U3, absorbed from `per-provider-quota-integration`)

Absorbed so a single change owns the whole capability. Contract in
`specs/provider-quota-capture/spec.md`. The endpoint and dashboard phases above
still do not read these observations; wiring them into the summary is U5.

- [x] 5.1 Persistence: `QuotaObservation` model and `QuotaObservationRecorderPort` in `rook-core`; `provider_quota_observations` store with indexes on runtime, connection, and `observed_at`.
- [x] 5.2 Migration: `V8__provider_quota_observations.sql`, versioned after the already-shipped `V6__oauth_client_credentials.sql` and `V7__combo_aliases.sql`.
- [x] 5.3 Parsers: documented OpenAI, Anthropic Messages, and Groq rate-limit headers, covering valid, absent, malformed, and reset forms.
- [x] 5.4 Adapters: capture headers on success and error responses before body consumption, best-effort, without altering provider outcomes.
- [x] 5.5 Canonical gate: enable capture for default and explicit canonical endpoints, disable for custom/proxy/non-HTTPS/port/foreign-host endpoints.
- [x] 5.6 Identity: restore managed runtime identity and originating connection through the recorder wrapper.
- [x] 5.7 Bounded persistence: queue with capacity 1024, non-blocking enqueue, drop counter, dedicated writer thread, drain-and-join on shutdown.
- [x] 5.8 Retention: bounded-batch deletion mirroring `usage_history` retention.
- [x] 5.9 Scope: carry `anthropic-workspace-id` as request-scoped provenance when present.
- [x] 5.10 Tests: field-level assertions for provider identity, metric, limit, remaining, and scope; canonical gate accepted/rejected matrices; identity wrapper round-trip through SQLite.
- [x] 5.11 Remove the inline `#[cfg(test)]` module from `apps/rook/src/di.rs`; cover it from the `quota_identity_tests` integration target instead.
- [x] 5.12 Wire observations into the quota summary endpoint (U5). Verified separately from Cortex-routed usage, local 429 counts, and local thresholds.

## Phase 6: Dashboard Upstream Reconciliation (U6)

- [x] 6.1 Decision of record: render captured upstream observations in a dedicated dashboard section; keep the local warning explicitly labeled as a local routed-traffic warning; never compare upstream values against local monitoring thresholds.
- [x] 6.2 API types for `upstreamObservations` in `apps/rook/dashboard/src/lib/api.ts` (availability, freshness, provenance, scope, window, limit/remaining, observation time, age, reset).
- [x] 6.3 View section `upstream-observations-<providerKind>` with metric, scope, window, limit, remaining, source, freshness, age and reset; `not_reported` renders an explicit empty state and `unavailable` renders no section.
- [x] 6.4 Localization in `en.json` and `es.json` under `providers.quota.upstream`, including the disclaimer that these are provider-reported observations and not Cortex activity or account-wide guarantees.
- [x] 6.5 Tests for `available`, `not_reported` and `unavailable` states in `ProvidersQuotaView.spec.ts`; verified with `pnpm --filter @dallay/rook-dashboard exec vitest run src/views/ProvidersQuotaView.spec.ts` (5 tests).

## Consolidation Note

`per-provider-quota-integration` was retired. Its scope (U1–U3) is Phase 5 of
this change; its design and exploration remain as reference under
`reference/`. U5 (endpoint/DTO wiring) is complete. U6 (dashboard reconciliation)
was resolved by the U6 decision of record: the dashboard renders captured upstream
observations in a dedicated section with provenance, scope/window, age and
freshness, and keeps the local warning explicitly labeled as a local
routed-traffic warning derived from recorded 429 ratios. Upstream observations
never drive the warning level and are never compared against the local
monitoring thresholds. The two designs no longer conflict: the reference
capture design is applied only to the observation display, while its
threshold-reuse proposal is rejected.

## Implementation order

Consolidate all phases into the same ordinary PR, in dependency order: SQLite aggregates, backend contract/endpoint, dashboard/localization and integrated verification. Do not omit technical criteria when consolidating. The explicit size exception for this single PR is approved, as stated in the authorization recorded above.
