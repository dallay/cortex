# Tasks: Per-Provider Quota Integration

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 650–950 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 storage; PR 2 capture/API; PR 3 dashboard/spec |
| Delivery strategy | feature-branch-chain (user selected) |
| Chain strategy | feature-branch-chain |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain
400-line budget risk: High

Apply gate: Unblocked; proceed once the integration and work-unit branches are created or confirmed. Keep units independently reviewable; no additional approval or GitHub Stack workflow is required.

### Suggested Work Units

Topology: integration branch `feature/dallay-404-per-provider-quota-integration-real-data-not-mock` branches from `main`. Unit PR bases: integration branch → Unit 1 → Unit 2. After verification, propose the integrated result to `main`. Feature-branch chain only; no GitHub Stack metadata. Source: Linear `DALLAY-404`; do not infer GitHub numbering or encode issue metadata.

| Unit | Goal | PR branch → base | Dependency |
|------|------|-----------------|------------|
| 1 | Domain model, V6 storage, repository and retention | `feature/dallay-404-quota-storage` → `feature/dallay-404-per-provider-quota-integration-real-data-not-mock` | Position 1; no work-unit dependency. |
| 2 | Verified response capture and distinct quota API | `feature/dallay-404-quota-capture-api` → `feature/dallay-404-quota-storage` | Position 2; depends on Unit 1. |
| 3 | Honest quota UI, contract and documentation | `feature/dallay-404-quota-dashboard-spec` → `feature/dallay-404-quota-capture-api` | Position 3; depends on Unit 2. |

Keep PRs autonomous; child diffs exclude earlier units. Create/confirm branches from stated bases.

## Phase 1: Observation Storage

- [x] 1.1 Add observation model/recorder port in `crates/domain/rook-core/src/{model.rs,ports.rs}` with source, metric, identity, scope/window, limit/remaining, reset and observation time.
- [x] 1.2 Add isolated table/indexes in `crates/infrastructure/db-migration/src/migrations/V6__provider_quota_observations.sql`; no backfill or `usage_history` changes.
- [x] 1.3 Implement SQLite persistence/quota-only `retention_days` cleanup in `crates/infrastructure/audit-sqlite/src/lib.rs`; wire recorder and existing sweep in `apps/rook/src/{di.rs,usage_retention.rs}`. Test migration, round-trip, identity, retention.

## Phase 2: Capture and API

- [ ] 2.1 Parse documented OpenAI, Anthropic Messages and Groq headers in provider adapters; preserve success/error metadata before stream consumption. Test valid/missing/malformed/reset cases.
- [ ] 2.2 Record best-effort without changing completion results; test response/error/stream and recorder failure. No polling or Anthropic Admin API calls.
- [ ] 2.3 Extend `crates/infrastructure/transport-axum/src/{handlers/provider_quota.rs,provider_dto.rs}` with distinct local usage/trend, upstream observations/age/expiry and named 429 counts. Test mapping, unavailable≠zero, freshness, no 429 warnings.
- [ ] 2.4 Classify per managed connection and compatible metric/window/scope using reported values and its thresholds; suppress invalid, unknown-expiry or threshold-less cases. Test 0.8/0.95 and incompatibilities.

## Phase 3: Dashboard and Contract

- [ ] 3.1 Update `apps/rook/dashboard/src/lib/api.ts` and `composables/useProvidersQuota.ts`; test deserialization, unavailable/stale/history.
- [ ] 3.2 Update `views/ProvidersQuotaView.vue` and `locales/{en,es}.json` for separate local trend/upstream history, per-connection warnings, age/expiry, unsupported/unavailable, 429 frequency. Test no-data/mixed-data.
- [ ] 3.3 Replace placeholder contract in `.agents/sdd/specs/providers-ui/spec.md`; verify no mocked/follow-up banner remains.

## Phase 4: Verification

- [ ] 4.1 Run focused Rust migration/repository, parser/capture and handler tests; dashboard `pnpm exec vitest run` and `pnpm exec vue-tsc --noEmit`.
- [ ] 4.2 Run `just ci-local`; if Docker E2E is unavailable, run `just ci-ci` and report E2E not run (`just test-e2e` required for full verification).
