# Tasks: Per-Provider Quota Integration

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | U1 250–350 (done); U2–5 180–390 each |
| 400-line budget risk | Medium; each unit forecast below 400 |
| Chained PRs recommended | Yes |
| Suggested split | Five units; U1 done, U2–5 remain |
| Delivery strategy | feature-branch-chain (user selected) |
| Chain strategy | feature-branch-chain |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain
400-line budget risk: Medium

**Stop/replan gate:** if any unit is forecast or reaches 400 changed lines, stop and subdivide/replan; no waiver. User authorizes two additional local signed commits (five units total). No push/PR.

### Suggested Work Units

Feature-branch chain only: integration branch `feature/dallay-404-per-provider-quota-integration-real-data-not-mock` → `main`; each unit bases on its predecessor. No GitHub Stack metadata. Linear `DALLAY-404`. U1 signed in `fb691f1`; U2 branch is clean, based on U1, no source changes.

| Unit | Scope; estimate | Branch → base | Position / dependency |
|------|----------------|---------------|----------------------|
| 1 | Model, V6 storage, repository/retention; 250–350 (done) | `feature/dallay-404-quota-storage` → integration | 1; `fb691f1` |
| 2 | OpenAI/Anthropic Messages/Groq parsing + tests; 180–260 | `feature/dallay-404-quota-capture-api` → U1 | 2; existing clean branch |
| 3 | Adapter capture, best-effort recording, stream/error + identity tests; 250–350 | `feature/dallay-404-quota-capture-wiring` → U2 | 3 |
| 4 | Quota endpoint/DTO, freshness, per-connection warnings + handler tests; 300–390 | `feature/dallay-404-quota-api` → U3 | 4 |
| 5 | Dashboard/tests + stale UI spec reconciliation; 280–380 | `feature/dallay-404-quota-dashboard-spec` → U4 | 5 |

Child diffs contain only their unit's changes; slices are local, not published PRs.

## Phase 1: Observation Storage — Unit 1 (Complete)

- [x] 1.1 Add model/recorder port, V6 table/indexes, SQLite retention/persistence and DI/sweep wiring in the approved domain, migration, infrastructure and app files. No backfill or `usage_history` changes.

## Phase 2: Provider Parsing — Unit 2

- [x] 2.1 Parse/normalize documented OpenAI, Anthropic Messages, Groq response headers; provider tests cover valid, absent, malformed, and reset values.

## Phase 3: Capture and Recording — Unit 3

- [ ] 3.1 Capture after send (errors too, before stream consumption); wire best-effort recording without changing results. Test recorder failure, streams and runtime/connection identity.

## Phase 4: Quota API — Unit 4

- [ ] 4.1 Map local usage/trend, upstream observations/age/expiry and named 429 counts in `crates/infrastructure/transport-axum/src/{handlers/provider_quota.rs,provider_dto.rs}`; test mapping, unavailable≠zero, freshness, thresholds, no 429 warnings.

## Phase 5: Dashboard and Contract — Unit 5

- [ ] 5.1 Update `apps/rook/dashboard/src/{lib/api.ts,composables/useProvidersQuota.ts,views/ProvidersQuotaView.vue,locales/{en,es}.json}` for separate local/upstream data, age/expiry and warnings; test stale, mixed, no-data.
- [ ] 5.2 Reconcile `.agents/sdd/specs/providers-ui/spec.md`; remove stale placeholder/follow-up text, preserving approved semantics.

## Phase 6: Verification

- [ ] 6.1 Run provider tests (`cargo test -p providers-openai -p providers-anthropic -p providers-groq`), app/handler tests (`cargo test -p rook -p transport-axum`), dashboard `cd apps/rook/dashboard && pnpm exec vitest run && pnpm exec vue-tsc --noEmit`.
- [ ] 6.2 Run `just ci-local`; if Docker E2E is unavailable, run `just ci-ci` and report E2E not run (`just test-e2e` required for full verification).

**Exact scope exclusions:** no polling; no Anthropic Admin API calls; no speculative Gemini/Ollama quota signals. No 429-derived utilization or incompatible aggregation; keep local/upstream separate.
