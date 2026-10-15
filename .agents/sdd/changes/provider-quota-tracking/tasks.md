# Tasks: Provider Quota Tracking

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Review budget | 400 changed lines |
| Estimated workload | High — backend/domain plus dashboard, broad behavior and test coverage; likely >400 changed lines |
| Chained PRs recommended | Yes |
| Chain strategy | **Decision required before apply:** choose `github-stacked-prs`, `feature-branch-chain`, or approved `size-exception`; no strategy selected yet |
| Work-unit balance | Three deliverable slices: cost completeness, truthful backend summary, then dashboard usage/refresh experience; each carries its tests and relevant user-facing copy. |

Decision needed before apply: Yes
Chained PRs recommended: Yes
400-line budget risk: High

**Blocker:** Before `sdd-apply`, the user must choose one canonical review strategy. If chained PRs are chosen, record its branch/base metadata and exact slice assignment before implementation. `size-exception` requires explicit approval and rationale. Issue #132 is the tracker reference; no branch or PR metadata is assumed here.

## Project Test Mode and Runners

- `.agents/sdd/config.yaml` sets `strict_tdd: false`; global RED→GREEN→REFACTOR is **not** enabled. Test-first is required below only for pure logic with a suitable seam (notably per-connection threshold classification); other tests are ordinary functional checks.
- Existing runners: Rust `cargo test` (configured command: `cargo test --workspace --all-features`); dashboard Vitest (`cd apps/rook/dashboard && pnpm exec vitest run`). No `.agents/sdd/quality-runner.json` or `scripts/sdd-quality-runner.mjs` is configured, so do not claim deterministic SDD runner enforcement.

## Phase 1: Cost Completeness — Work Unit 1

- [ ] 1.1 Extend `CostBreakdown` in `crates/domain/rook-core/src/model.rs` with known/unknown cost-entry counts while preserving existing numeric total semantics for current consumers.
- [ ] 1.2 Update `SqliteUsageRepository::cost_breakdown` in `crates/infrastructure/audit-sqlite/src/lib.rs` to aggregate `SUM(cost_usd)`, `COUNT(cost_usd)`, and matching-entry count under the existing filters; verify explicit zero, priced, unknown, mixed, and empty windows, including connection/date filters.
- [ ] 1.3 Update affected `UsageRecorderPort` mocks/test doubles only where required by the aggregate contract; run focused audit-sqlite and dependent crate tests.

## Phase 2: Truthful Backend Summary — Work Unit 2 (depends on 1)

- [ ] 2.1 Extract a pure per-connection threshold classifier in `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs`; RED: add test-target cases for warning/error boundaries and unavailable inputs, then GREEN: implement minimum classification and REFACTOR without behavior change.
- [ ] 2.2 Replace legacy quota DTO fields in `crates/infrastructure/transport-axum/src/provider_dto.rs` with explicit coverage, availability, cost completeness, `accountQuota`, grouped local thresholds, rate-limit observation, warning level, periods, and trend fields; preserve `GET /api/providers/quota` and camelCase serialization.
- [ ] 2.3 In `provider_quota.rs`, construct rows for all six provider kinds from listed connections; expose Cortex-only coverage, configuration status, grouped thresholds, and correct Ollama local/Cloud `accountQuota` availability.
- [ ] 2.4 In `provider_quota.rs`, map all-time/24-hour/7-day and seven UTC trend-bucket measures to explicit availability/cost states; isolate recorder failures per measure/window so successful data survives and unknown cost stays null.
- [ ] 2.5 Add/extend `crates/infrastructure/transport-axum/tests/` route tests using fake recorder and `ManageConnections`; cover spec scenarios for partial failures, DTO serialization, provider rows, cost states, threshold provenance, and missing core dependencies. Run focused Rust tests.

## Phase 3: Dashboard Usage and Refresh — Work Unit 3 (depends on 2)

- [ ] 3.1 Mirror backend enums, nullable costs, and response shape in `apps/rook/dashboard/src/lib/api.ts`; keep `getProvidersQuota()` URL unchanged.
- [ ] 3.2 Extend `useProvidersQuota.ts` and `useProvidersQuota.spec.ts` for manual/60-second refresh, overlap suppression, retained last-good response, stale/recovery state, initial failure, and unmount timer cleanup; run focused Vitest.
- [ ] 3.3 Update `ProvidersQuotaView.vue` and `ProvidersQuotaView.spec.ts` for coverage/provenance, unavailable vs no-data, local warnings, account-quota states, unknown costs/providers, and stale refresh affordance.
- [ ] 3.4 Render seven-day requests/tokens and known estimated cost with existing chart primitives; add accessible equivalent and chart empty/unavailable assertions in `ProvidersQuotaView.vue` and its spec.
- [ ] 3.5 Update `apps/rook/dashboard/src/locales/en.json`, `apps/rook/dashboard/src/locales/es.json`, and `src/__tests__/providersI18n.spec.ts`; verify English/Spanish labels and run dashboard Vitest, typecheck, and build.

## Phase 4: Integrated Verification

- [ ] 4.1 Check both delta specs against backend and dashboard tests; run focused Rust/dashboard checks, then repository gate `just ci-local` before reporting completion.
- [ ] 4.2 Confirm no change to `V2__usage_history.sql`, provider adapters, routing, DI, admin authorization, or `/providers/quota`; record any unavoidable scope deviation for verification.

## Work-Unit Boundaries (conditional on strategy decision)

1. **Usage cost completeness:** tasks 1.1–1.3; starts from existing nullable `cost_usd`, ends with verified completeness aggregates; rollback is isolated to the aggregate contract/implementation/tests.
2. **Backend usage contract:** tasks 2.1–2.5; depends on Work Unit 1, ends with verified endpoint DTO/partial-availability behavior; rollback leaves the prior dashboard contract untouched until this unit is integrated.
3. **Dashboard usage experience:** tasks 3.1–3.5; depends on Work Unit 2, ends with localized, refreshable and accessible usage display; rollback preserves the backend endpoint.

## Implementation Order

Complete and verify Work Unit 1, then the endpoint contract in Work Unit 2, then dashboard Work Unit 3, followed by integrated verification. Do not start apply until the review-strategy blocker above is resolved; keep each unit's behavior, tests, and user-facing copy together.
