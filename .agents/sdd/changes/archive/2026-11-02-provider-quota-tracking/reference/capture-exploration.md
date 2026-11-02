# Exploration: DALLAY-404 — Per-provider quota integration (real data, not mock)

**Status:** explored; ready for product decisions before proposal/spec.

## Executive summary

The issue’s “mock data” premise is stale in the current checkout: `GET /api/providers/quota` already aggregates real locally recorded usage from the SQLite-backed usage recorder, and the dashboard fetches that API. The backend currently reports request/token/cost history, rate-limited responses, and threshold-derived warning levels—not provider-reported quota headroom. It contains provider-specific caveats, including that remaining limits are unavailable/not persisted. The page itself has no development banner or mock fixture; the active `providers-ui` spec still describes the old placeholder and banner.

The substantial gap is therefore semantic and coverage, not replacing a mock wholesale: decide whether this feature means local observed usage, upstream-reported limits when available, or both as clearly distinguished measures. Historical request usage already exists; persisted samples of provider-reported headroom do not. Groq parses rate-limit headers for error handling but does not retain successful-response headers for this endpoint. No quota-specific dashboard or transport tests were found in the searched paths.

## Current State

- `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs` serves `/api/providers/quota`. It loads provider connections, queries the `UsageRecorderPort` for all-time, last-24-hour, last-7-day windows, and builds a seven-day daily trend. Aggregates include requests, tokens, recorded rate-limited requests, and cost.
- The source history is `usage_history` (`crates/infrastructure/db-migration/src/migrations/V2__usage_history.sql`) populated through `UsageRecorderPort`; this is real per-request local history, not a separately polled quota snapshot. `UsageFilters` supports connection/provider/date/status filters. The quota handler currently aggregates via configured connection IDs, so usage with no matching connection ID (e.g. TOML-only/best-effort connection propagation) may not appear in these per-kind aggregates.
- “Warning” currently means the observed share of rate-limited requests over the last seven days compared against connection-configured warning/error fractions. It is not a percentage of upstream quota consumed or remaining. Thresholds are averaged across connections of a provider kind, which can mask different per-connection limits.
- The frontend (`ProvidersQuotaView.vue` + `useProvidersQuota.ts` + `lib/api.ts`) fetches live API data on mount and displays support notes, 24-hour tokens, seven-day cost, observed rate-limit ratio, and status. The endpoint DTO includes trends, but the page does not currently render the trend. The composable does not poll/refresh automatically.
- Provider evidence in current code: OpenAI and Anthropic adapters read `Retry-After` and `x-ratelimit-reset` when mapping error responses; normal-response headers are not retained by the quota path. Groq similarly parses error headers, while successful response headers are discarded. Gemini parses `usageMetadata` into token usage, with no quota metadata model in the inspected adapter. Ollama handles optional cloud `X-RateLimit-*` on errors/probes, not as persisted quota telemetry. These establish current integration behavior only; vendor support/availability statements should be checked against current official provider documentation during design.
- The page has no development banner in the current Vue view, and no quota-specific mock source was found in the dashboard source search. However `.agents/sdd/specs/providers-ui/spec.md` still specifies mock data and a follow-up banner (lines 122–131), while `_metadata.md` calls quota implementation a non-goal/placeholder. Those docs conflict with current implementation and issue acceptance.
- Coverage: no `ProvidersQuotaView` test file or quota-specific transport test was found in the searched paths. Existing generic provider route tests cover provider CRUD/DTO behavior, not the quota aggregation contract.

## Affected Areas

- `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs` — current aggregation, inferred warning status, provider notes, and trends.
- `crates/infrastructure/transport-axum/src/provider_dto.rs` — quota API response DTOs.
- `crates/infrastructure/transport-axum/src/routes.rs` — endpoint registration at `/api/providers/quota`.
- `crates/infrastructure/providers-{openai,anthropic,groq,gemini,ollama}/src` — response/usage/header extraction boundaries; adapters currently do not expose generalized provider quota observations to the quota endpoint.
- `crates/domain/rook-core` usage ports/types and `crates/infrastructure/audit-sqlite` — local usage history source and filter semantics.
- `crates/infrastructure/db-migration/src/migrations/V2__usage_history.sql` — existing persisted request-level history; no upstream quota snapshot table identified.
- `apps/rook/dashboard/src/views/ProvidersQuotaView.vue`, `src/composables/useProvidersQuota.ts`, and `src/lib/api.ts` — API display, refresh behavior, and wire types.
- `.agents/sdd/specs/providers-ui/spec.md` and `.agents/sdd/specs/usage-history/spec.md` — reconcile old placeholder acceptance with actual usage-history behavior and new semantics.

## Approaches

1. **Local observed usage as the supported baseline (recommended for a narrow first scope)** — keep SQLite request history as the source for usage/trends; correct labels/status semantics, present per-provider capability/availability honestly, use graceful “not reported/not supported” states, and remove/update stale placeholder docs. Add upstream quota observation only where the adapter can reliably surface documented response metadata.
    - Pros: builds on verified persisted data; works for providers with no account-level quota API; historical data is already available; bounded scope and graceful degradation.
    - Cons: cannot truthfully show remaining account/workspace quota for providers without a reported limit; changing warning semantics may affect dashboard meaning; local usage can diverge from provider-side usage.
    - Effort: Medium.

2. **Provider-reported headroom plus separate local usage/history** — introduce a typed quota-observation model (e.g. metric, value/limit/remaining, window, observed time, source/freshness), capture provider response headers/metadata in adapters, persist observations, expose them via the endpoint, and show reported headroom separately from local usage. Providers without a source return an explicit unavailable state.
    - Pros: preserves semantic distinction; allows genuine headroom/warnings where the provider reports it; persistent observations enable historical quota trends.
    - Cons: provider- and metric-specific support; successful streaming headers need capture before body consumption; headers may be absent, reset by connection/workspace, or have provider-defined units/windows; migrations, retention, DTO, adapter, and UI changes; no guarantee this provides true account-level quotas for all providers.
    - Effort: High.

3. **Treat local usage as an estimate of provider quota consumption** — aggregate request/token/cost records and configure/derive limits, then calculate utilization and warnings.
    - Pros: can give a unified utilization UI and historical chart without provider APIs.
    - Cons: limits are not currently known per provider/account; configured rate-limit warning fractions currently apply to local 429 ratio, not consumption; estimates are liable to mislead and do not equal provider-reported quota.
    - Effort: Medium, with substantial product/accuracy risk.

## Recommendation

Before proposing implementation, define “quota” as two distinct data classes: **locally observed usage** (already available from `usage_history`) and **provider-reported limit/headroom** (optional, sourced only when available). Prefer approach 2 if DALLAY-404 acceptance requires true provider-reported quota and persisted history of it; if the intent is primarily a useful real-data page, choose approach 1 and explicitly avoid presenting local usage or rate-limit ratios as upstream quota utilization. Do not treat issue-suggested provider capabilities as verified guarantees; validate against current official provider docs for the specific metrics and response headers before committing acceptance criteria.

## Open Product Questions

- Does “historical data over time” mean existing local request usage history/trends (already stored) or history of provider-reported remaining/limit snapshots (not identified in the current schema)? What retention and granularity are expected?
- Should the UI show separate **observed usage**, **429/rate-limit events**, and **reported quota/headroom** fields, or is one of these the actual goal?
- What does “warnings near limits” mean: local configured thresholds, observed 429 frequency, or provider-reported remaining quota? What windows and per-connection vs per-provider aggregation are intended?
- Should the endpoint/page include TOML/runtime providers that lack a persisted `ConnectionId`, as well as inactive connections? Current grouping derives connection IDs from the managed-connection list.
- Must automatic polling be added, or is load-on-navigation/manual refresh sufficient? If polling is required, specify cadence and stale/error presentation.
- Is the “development banner” a requested removal of a still-live banner, or simply stale acceptance text? The current inspected page has no such banner.
- What is expected when there are no usage rows, no configured connections, or provider metadata is stale/unavailable?

## Risks

- Labeling locally observed usage or 429 share as provider quota utilization would create misleading operational decisions.
- Provider header/API contracts differ by provider, account, metric (requests/tokens), plan, and time window; a generic normalized quota can lose meaning.
- Current threshold averaging and provider-kind aggregation can obscure connection-specific limits; current handler also selects by connection IDs, potentially excluding unassociated usage.
- Persisting quota observations raises migration, retention, freshness, and concurrent request/stream response-capture concerns.
- The implementation/spec drift means acceptance text must be updated rather than assuming the current code matches the mock-page spec.
- Lack of focused endpoint/UI tests leaves aggregation, no-data, unavailable-provider, and warning semantics insufficiently pinned down.

## Ready for Proposal

**Not yet.** Ask the product owner to resolve the first three semantic questions (reported quota vs local usage, historical data type, warning basis), and clarify whether polling is required. Once those are decided, proposal can scope adapter capture and persistence only to documented provider signals and preserve explicit unavailable states elsewhere.
